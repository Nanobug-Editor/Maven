//! Current Maven models project into generic Java launch contexts and ordinary Maven build steps.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    configurations as contract, service,
};
use serde_json::json;
use std::{cell::RefCell, collections::BTreeMap};

/// Deferred validation retains the original source request and its exact Java provider reference.
struct Pending {
    reply: api::ResourceHandle,
    reference: api::ResourceHandle,
    task: service::guest::Task,
    launch: contract::Launch,
}
thread_local! {static PENDING:RefCell<BTreeMap<u64,Pending>>=const{RefCell::new(BTreeMap::new())};}

/// Saving and executing use the same current model; validation performs no Maven build or JVM launch.
pub(crate) fn validate(
    state: &crate::imports::State,
    call: service::Invocation,
) -> Result<api::Output, Failure> {
    let result = (|| {
        let values: crate::configurations::Values =
            serde_json::from_str(call.arguments["values"].as_str().unwrap_or(""))
                .map_err(invalid)?;
        if let Some(problem) = values
            .command
            .problem(call.arguments["locale"].as_str().unwrap_or("zh-CN"))
        {
            return Err(invalid(problem));
        }
        let (model, generation) = state.current_model()?;
        let options = values.options();
        options.validate()?;
        if serde_json::to_value(&options).map_err(invalid)? != model["request"] {
            return Err(invalid(
                "先按此 POM／profile 导入工程 / Import this POM/profile selection first",
            ));
        }
        let modules = model["modules"]
            .as_array()
            .ok_or_else(|| invalid("Missing Maven modules"))?;
        let module = modules
            .iter()
            .find(|module| module["id"] == values.module && module["packaging"] != "pom")
            .ok_or_else(|| invalid("请选择导入的模块 / Select an imported module"))?;
        let mut selected = crate::model::runtime_project(module, modules)?;
        selected.java["main_class"] = json!(values.main_class);
        // Runtime entries already contain the resolved JARs. Avoid duplicating a large classpath
        // in inert metadata; Java reconstructs its private debug index from those same entries.
        selected.java["jars"] = json!([]);
        let project = json!({"project":selected.java,"jdk":module["jdk"].as_str().unwrap_or("").strip_prefix("1.").unwrap_or(module["jdk"].as_str().unwrap_or("")),"classpath":selected.classpath});
        let reference = service::guest::open("java.project")?;
        let started = service::guest::Task::start(
            &reference,
            "launch_metadata",
            json!({"project":project.to_string(),"key":format!("maven:{}",values.module),"generation":generation,"main_class":values.main_class}),
            120_000,
        );
        let task = match started {
            Ok(task) => task,
            Err(error) => {
                let _ = api::guest::close_resource(reference);
                return Err(error);
            }
        };
        let mut launch = values
            .command
            .launch("java", call.arguments["workspace"].as_str().unwrap_or(""));
        let mut args = vec![
            "-pl".to_owned(),
            values
                .module
                .split(':')
                .take(2)
                .collect::<Vec<_>>()
                .join(":"),
            "-am".into(),
            "compile".into(),
        ];
        // Maven evaluates the reactor's own build plugins and resources before Java uses its outputs.
        if options.offline {
            args.push("-o".into());
        }
        launch.build = vec![
            json!({"name":"Maven compile","target":{"mode":"provided","provider":"maven","label":"Maven compile","binding":json!({"schema":1,"pom":options.pom,"profiles":options.profiles,"offline":options.offline}).to_string(),"args":args}}),
        ];
        launch.target = json!({"mode":"provided","provider":"java","label":values.command.name,"binding":"","args":values.command.args});
        launch.provider = Some("java".into());
        let reply = call
            .reply
            .clone()
            .ok_or_else(|| invalid("Missing configuration reply"))?;
        PENDING.with_borrow_mut(|pending| {
            pending.insert(
                reply.resource,
                Pending {
                    reply,
                    reference,
                    task,
                    launch,
                },
            );
        });
        Ok(())
    })();
    Ok(match result {
        Ok(()) => Default::default(),
        Err(error) => api::Output {
            service_reply: Some(Ok(
                json!({"payload":serde_json::to_string(&contract::Validation{valid:false,message:error.message,launch:None}).map_err(invalid)?}),
            )),
            ..Default::default()
        },
    })
}

/// Settle only the original configuration validation; cancellation never becomes a fresh launch.
pub(crate) fn event(event: &api::Notification) {
    let api::Notification::Service(event) = event else {
        return;
    };
    PENDING.with_borrow_mut(|pending| {
        if let service::Notification::InvocationCancelled { request, .. } = event
            && let Some(call) = pending.remove(&request.resource)
        {
            let _ = call.task.cancel(api::CancelMode::StopWaiting);
            let _ = api::guest::close_resource(call.reference);
        }
        let ended = pending
            .iter_mut()
            .filter_map(|(id, call)| match call.task.update(event) {
                Some(api::RequestUpdate::Completed { result }) => Some((*id, result)),
                Some(api::RequestUpdate::Cancelled { reason, .. }) => Some((
                    *id,
                    Err(Failure::new(
                        reason,
                        "Java application validation cancelled",
                    )),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, result) in ended {
            let mut call = pending.remove(&id).unwrap();
            let _ = api::guest::close_resource(call.reference);
            let mut validation = match result {
                Ok(result) => {
                    call.launch.target["binding"] = result["binding"].clone();
                    for entry in result["env"].as_array().into_iter().flatten() {
                        if let (Some(name), Some(value)) =
                            (entry["name"].as_str(), entry["value"].as_str())
                        {
                            call.launch.env.insert(name.into(), value.into());
                        }
                    }
                    contract::Validation {
                        valid: true,
                        message: String::new(),
                        launch: Some(call.launch),
                    }
                }
                Err(error) => contract::Validation {
                    valid: false,
                    message: error.message,
                    launch: None,
                },
            };
            if validation
                .launch
                .as_ref()
                .is_some_and(|launch| launch.env.len() > 64)
            {
                validation = contract::Validation {
                    valid: false,
                    message: "运行环境超过 64 项 / Launch environment exceeds 64 entries".into(),
                    launch: None,
                };
            }
            let result = serde_json::to_string(&validation)
                .map(|payload| json!({"payload":payload}))
                .map_err(invalid)
                .and_then(|envelope| {
                    if serde_json::to_vec(&envelope).map_err(invalid)?.len() > 64 * 1024 {
                        Err(Failure::new(
                            ErrorCode::LimitExceeded,
                            "Java launch configuration exceeds the public response budget",
                        ))
                    } else {
                        Ok(envelope)
                    }
                });
            let _ = service::guest::reply(&call.reply, result);
        }
    });
}

/// Configuration problems retain an actionable message without starting the selected target.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
