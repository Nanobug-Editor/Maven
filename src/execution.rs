//! Target preparation resolves an owned Maven distribution; the actual goal runs in a native session.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    process, service,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};
/// A short metadata probe retains the exact result-bearing target call and literal launch metadata.
struct Pending {
    process: api::ResourceHandle,
    reply: api::ResourceHandle,
    program: String,
    args: Vec<String>,
    bytes: Vec<u8>,
}
thread_local! {static PENDING:RefCell<BTreeMap<u64,Pending>>=const {RefCell::new(BTreeMap::new())};}
/// Portable bindings contain no tool path, executable source, credentials or extra authority.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    schema: u32,
    pom: String,
    profiles: Vec<String>,
    offline: bool,
}

/// Resolve only this contributor's original workspace and bounded POM choices.
pub(crate) fn invoke(
    state: &crate::imports::State,
    call: service::Invocation,
) -> Result<api::Output, Failure> {
    let result = (|| {
        if call.method == "discover" {
            return Ok(Some(json!({"targets":[]})));
        }
        if call.method != "prepare" {
            return Err(invalid("Unsupported Maven target method"));
        }
        let binding: Binding =
            serde_json::from_str(call.arguments["binding"].as_str().unwrap_or(""))
                .map_err(invalid)?;
        if binding.schema != 1 {
            return Err(invalid("Unsupported Maven target binding"));
        }
        let options = crate::imports::Options {
            pom: binding.pom,
            profiles: binding.profiles,
            offline: binding.offline,
        };
        options.validate()?;
        let reply = call
            .reply
            .clone()
            .ok_or_else(|| invalid("Missing Maven target reply"))?;
        let mut args = state.native_arguments(options, reply.resource)?;
        args.extend(["--mode".into(), "goal".into()]);
        let mut probe = args.clone();
        *probe.last_mut().unwrap() = "describe".into();
        let program = process::inspect_tool(
            process::ToolSource::Service {
                service: "maven".into(),
            },
            0,
            vec![],
        )?
        .program;
        let process = crate::imports::start_native(&probe)?;
        PENDING.with_borrow_mut(|pending| {
            pending.insert(
                process.resource,
                Pending {
                    process,
                    reply,
                    program,
                    args,
                    bytes: vec![],
                },
            )
        });
        Ok(None)
    })();
    Ok(match result {
        Ok(None) => Default::default(),
        Ok(Some(value)) => api::Output {
            service_reply: Some(Ok(value)),
            ..Default::default()
        },
        Err(error) => api::Output {
            service_reply: Some(Err(error)),
            ..Default::default()
        },
    })
}
/// Only complete native metadata and a successful probe may become a literal prepared launch.
pub(crate) fn event(event: &api::Notification) {
    PENDING.with_borrow_mut(|pending| {
        if let api::Notification::Service(service::Notification::InvocationCancelled {request,..})=event {
            let keys=pending.iter().filter(|(_,item)|item.reply==*request).map(|(key,_)|*key).collect::<Vec<_>>();
            for key in keys {if let Some(item)=pending.remove(&key){let _=api::guest::close_resource(item.process);}}
        }
        let api::Notification::Process {handle,update}=event else{return;};
        let Some(item)=pending.get_mut(&handle.resource).filter(|item|item.process==*handle) else{return;};
        let result=match update {
            process::Update::Output {stream:process::Stream::Stdout,bytes}=>{
                if item.bytes.len()+bytes.len()<=8192 {item.bytes.extend(bytes);return;}
                Err(invalid("Maven metadata exceeds its quota"))
            },
            process::Update::Output {..}=>return,
            process::Update::Exited {code:0}=>serde_json::from_slice::<Value>(&item.bytes).map_err(invalid).and_then(|metadata| {
                let home=metadata["home"].as_str().filter(|home|!home.is_empty()).ok_or_else(||invalid("Missing prepared Maven home"))?;
                let args=serde_json::to_string(&item.args).map_err(invalid)?;
                if args.len()>4096 {return Err(invalid("Maven startup metadata exceeds the target argument budget"));}
                Ok(json!({"program":item.program,"prefix_args":["--home",home,"--metadata",args,"--"]}))
            }),
            process::Update::Exited {code}=>Err(Failure::new(ErrorCode::OperationFailed,format!("Maven preparation exited with code {code}"))),
            process::Update::Terminated=>Err(Failure::new(ErrorCode::Cancelled,"Maven preparation terminated")),
        };
        let item=pending.remove(&handle.resource).unwrap();
        let _=api::guest::close_resource(item.process);
        let _=service::guest::reply(&item.reply,result);
    });
}
/// Target errors never reuse an earlier prepared executable as a successful result.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
