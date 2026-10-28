//! Maven supplies a resolved reactor graph; Java owns its shared analysis process and bindings.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    service,
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};

/// Deferred calls retain their exact provider reference until completion or source cancellation.
struct Pending {
    reply: api::ResourceHandle,
    reference: api::ResourceHandle,
    task: service::guest::Task,
}
thread_local! {static PENDING:RefCell<BTreeMap<u64,Pending>>=const {RefCell::new(BTreeMap::new())};}

/// A readonly language demand consumes a successful import, and never launches Maven implicitly.
pub(crate) fn invoke(
    state: &crate::imports::State,
    call: service::Invocation,
) -> Result<api::Output, Failure> {
    let result = (|| {
        let reply = call
            .reply
            .clone()
            .ok_or_else(|| invalid("A deferred language reply is required"))?;
        let (report, generation) = state.current_model()?;
        let path = call.arguments["document"]["path"]
            .as_str()
            .ok_or_else(|| invalid("Missing Java source"))?;
        let modules = report["modules"]
            .as_array()
            .ok_or_else(|| invalid("Invalid imported modules"))?;
        let _module = modules
            .iter()
            .filter(|module| module["packaging"] != "pom")
            .filter(|module| {
                ["source_roots", "test_source_roots"]
                    .into_iter()
                    .any(|field| {
                        module[field]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .any(|root| inside(path, root))
                    })
            })
            .max_by_key(|module| module["root"].as_str().unwrap_or("").len())
            .ok_or_else(|| {
                Failure::new(
                    ErrorCode::NotFound,
                    "The Java source is outside the imported Maven modules",
                )
            })?;
        let (project, jdk) = crate::model::analysis(modules)?;
        let reference = service::guest::open("java.project")?;
        let started = service::guest::Task::start(
            &reference,
            "attach_metadata",
            json!({
                "document":call.arguments["document"],"project":project.to_string(),"jdk":jdk,
            "key":format!("maven:{}",report["request"]["pom"].as_str().unwrap_or("pom.xml")),"generation":generation
            }),
            120_000,
        );
        let task = match started {
            Ok(task) => task,
            Err(error) => {
                let _ = api::guest::close_resource(reference);
                return Err(error);
            }
        };
        PENDING.with_borrow_mut(|pending| {
            pending.insert(
                reply.resource,
                Pending {
                    reply,
                    reference,
                    task,
                },
            )
        });
        Ok(())
    })();
    Ok(match result {
        Ok(()) => Default::default(),
        Err(error) => api::Output {
            service_reply: Some(Err(error)),
            ..Default::default()
        },
    })
}
/// Path membership is segment-boundary based, independent of source text and current editor focus.
fn inside(path: &str, root: &str) -> bool {
    root.is_empty()
        || root == "."
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Retire the borrowed service reference together with its original deferred reply.
pub(crate) fn event(event: service::Notification) {
    PENDING.with_borrow_mut(|pending| {
        if let service::Notification::InvocationCancelled { request, .. } = &event {
            if let Some(item) = pending.remove(&request.resource) {
                let _ = item.task.cancel(api::CancelMode::StopWaiting);
                let _ = api::guest::close_resource(item.reference);
            }
        }
        let mut completed = Vec::new();
        for (key, item) in pending.iter_mut() {
            let result = match item.task.update(&event) {
                Some(api::RequestUpdate::Completed { result }) => Some(result),
                Some(api::RequestUpdate::Cancelled { reason, .. }) => Some(Err(Failure::new(
                    reason,
                    "Imported Java preparation cancelled",
                ))),
                _ => None,
            };
            if let Some(result) = result {
                completed.push((*key, result));
            }
        }
        for (key, result) in completed {
            let item = pending.remove(&key).unwrap();
            let _ = service::guest::reply(&item.reply, result);
            let _ = api::guest::close_resource(item.reference);
        }
    });
}
/// Closed contract failures never silently select the ordinary Java source project.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
