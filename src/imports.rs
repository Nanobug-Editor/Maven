//! Deferred imports retain the exact native command's original reply and bounded output.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    commands, process, settings,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Host-owned effective choices apply before commands; live handles never enter logical snapshots.
#[derive(Default)]
pub(crate) struct State {
    workspace: String,
    settings: settings::Effective,
    pending: BTreeMap<u64, Import>,
    /// Last successful logical model remains visible when a newer import fails.
    model: Option<Value>,
    stale: bool,
    /// A late result may finish its own wait, but cannot publish over a newer import intent.
    intent: u64,
    panel: crate::panel::ProjectPanel,
    active: bool,
    /// One ordinary workspace subscription observes build inputs; it owns no editor text.
    watch: Option<(api::ResourceHandle, api::ResourceHandle)>,
}
/// Stdout is a private model channel; Maven's ordinary logs stay on stderr and in native history.
struct Import {
    process: api::ResourceHandle,
    reply: Option<api::ResourceHandle>,
    bytes: Vec<u8>,
    errors: Vec<u8>,
    intent: u64,
    options: Options,
}
/// Only logical, workspace-bound metadata is durable; no resource or invocation identity is restored.
#[derive(Deserialize, Serialize)]
struct Saved {
    workspace: String,
    model: Option<Value>,
}
/// Literal metadata selects a POM/profile set, never an executable, shell fragment or private path.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    pub(crate) pom: String,
    pub(crate) profiles: Vec<String>,
    pub(crate) offline: bool,
}

impl Options {
    /// Reject path escapes and malformed profile tokens before tool resolution or native execution.
    pub(crate) fn validate(&self) -> Result<(), Failure> {
        if self.pom.is_empty()
            || self.pom.contains(':')
            || self.pom.starts_with(['/', '\\'])
            || self
                .pom
                .replace('\\', "/")
                .split('/')
                .any(|part| part == ".." || part.is_empty())
            || self.profiles.len() > 32
            || self.profiles.iter().any(|profile| {
                profile.is_empty()
                    || profile.len() > 128
                    || profile.chars().any(|ch| ch.is_control() || ch == ',')
            })
        {
            return Err(invalid("Invalid Maven POM or profiles"));
        }

        Ok(())
    }
}

impl State {
    /// Only a completed current import may supply a language binding; failed/restored facts remain descriptive.
    pub(crate) fn current_model(&self) -> Result<(&Value, u64), Failure> {
        if self.stale {
            return Err(Failure::new(
                ErrorCode::StaleRevision,
                "Refresh the Maven project before Java analysis",
            ));
        }
        self.model
            .as_ref()
            .map(|model| (model, self.intent))
            .ok_or_else(|| {
                Failure::new(
                    ErrorCode::NotFound,
                    "Import the Maven project before Java analysis",
                )
            })
    }
    /// Finish only the original process and command; unrelated notifications cannot consume a reply.
    pub(crate) fn dispatch(&mut self, input: api::Input) -> Result<api::Output, Failure> {
        if let api::Input::Event { event, .. } = &input {
            crate::execution::event(event);
            crate::application::event(event);
        }
        match input {
            api::Input::Activate => self.active = true,
            api::Input::Event {
                panel: Some(panel),
                event: api::Notification::Ui(event),
            } if panel == "project" => {
                let result = if event.node == "maven-cancel"
                    && matches!(event.action, plugin_protocol::ui::Action::Click)
                {
                    self.cancel_imports("导入已取消 / Import cancelled", ErrorCode::Cancelled);
                    Ok(())
                } else {
                    self.panel.edit(event).and_then(|options| match options {
                        Some(options) => self.begin_options(options, None),
                        None => Ok(()),
                    })
                };
                if let Err(error) = result {
                    self.panel.status(error.message);
                }
            }
            api::Input::Event {
                event:
                    api::Notification::Service(plugin_protocol::service::Notification::Invoke(call)),
                ..
            } => {
                return match call.contract.as_str() {
                    "run.configurations" => crate::configurations::invoke(self, call),
                    "run.targets" => crate::execution::invoke(self, call),
                    _ => crate::language::invoke(self, call),
                };
            }
            api::Input::Event {
                event: api::Notification::Service(event),
                ..
            } => crate::language::event(event),
            api::Input::Prepare {
                environment,
                snapshot,
                ..
            } => {
                self.workspace = environment.workspace;
                self.model = snapshot
                    .filter(|saved| saved.schema == 1)
                    .and_then(|saved| serde_json::from_str::<Saved>(&saved.data).ok())
                    .filter(|saved| saved.workspace == self.workspace)
                    .and_then(|saved| saved.model);
                // Restored facts are descriptive until a fresh import proves the current inputs.
                self.stale = self.model.is_some();
            }
            api::Input::Snapshot => {
                return Ok(api::Output {
                    snapshot: Some(plugin_protocol::Snapshot {
                        schema: 1,
                        data: serde_json::to_string(&Saved {
                            workspace: self.workspace.clone(),
                            model: self.model.clone(),
                        })
                        .map_err(invalid)?,
                    }),
                    ..Default::default()
                });
            }
            api::Input::Event {
                event:
                    api::Notification::Configuration {
                        phase: settings::Phase::Apply,
                        values,
                    },
                ..
            } => {
                let changed =
                    serde_json::to_vec(&self.settings).ok() != serde_json::to_vec(&values).ok();
                self.settings = values;
                if changed && self.active && (self.model.is_some() || !self.pending.is_empty()) {
                    self.cancel_imports(
                        "Maven 配置已改变，请刷新 / Maven configuration changed; refresh required",
                        ErrorCode::StaleRevision,
                    );
                }
            }
            api::Input::Event {
                event:
                    api::Notification::WorkspaceFile {
                        subscription,
                        event,
                    },
                ..
            } if self
                .watch
                .as_ref()
                .is_some_and(|(_, owned)| *owned == subscription) =>
            {
                use plugin_protocol::workspace::FileEventKind as Kind;
                let changed = match event.kind {
                    Kind::Created { path } | Kind::Modified { path } | Kind::Deleted { path } => {
                        self.is_model_input(&path)
                    }
                    Kind::Renamed { source, target } => {
                        self.is_model_input(&source) || self.is_model_input(&target)
                    }
                    Kind::RescanRequired => true,
                };
                if changed {
                    self.cancel_imports(
                        "工程文件已改变，请刷新 / Project files changed; refresh required",
                        ErrorCode::StaleRevision,
                    );
                }
            }
            api::Input::Event {
                event: api::Notification::SubscriptionFailed { subscription, .. },
                ..
            } if self
                .watch
                .as_ref()
                .is_some_and(|(_, owned)| *owned == subscription) =>
            {
                if let Some((root, watch)) = self.watch.take() {
                    let _ = api::guest::close_resource(watch);
                    let _ = api::guest::close_resource(root);
                }
                self.cancel_imports(
                    "工程观察已中断，请刷新 / Project observation interrupted; refresh required",
                    ErrorCode::StaleRevision,
                );
            }
            api::Input::Event {
                event: api::Notification::CommandInvocation(call),
                ..
            } => {
                if call.id == "model" {
                    let payload=serde_json::to_string(&json!({"model":self.model,"stale":self.stale,"importing":!self.pending.is_empty()})).map_err(invalid)?;
                    return Ok(api::Output {
                        service_reply: Some(Ok(json!({"payload":payload}))),
                        ..Default::default()
                    });
                }
                if let Err(error) = self.begin(&call) {
                    return Ok(api::Output {
                        service_reply: Some(Err(error)),
                        ..Default::default()
                    });
                }
            }
            api::Input::Event {
                event: api::Notification::Process { handle, update },
                ..
            } => self.process(&handle, update),
            api::Input::Event {
                event: api::Notification::CommandCancelled { request, reason },
                ..
            } => {
                let cancelled = self
                    .pending
                    .iter()
                    .filter(|(_, pending)| pending.reply.as_ref() == Some(&request))
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>();
                for key in cancelled {
                    if let Some(pending) = self.pending.remove(&key) {
                        let _ = api::guest::close_resource(pending.process);
                        if pending.intent == self.intent {
                            self.panel.status(format!(
                                "导入已取消 / Import cancelled: {:?}",
                                reason.code
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(api::Output {
            views: if self.active {
                vec![
                    self.panel
                        .view(self.model.as_ref(), self.stale, !self.pending.is_empty()),
                ]
            } else {
                vec![]
            },
            ..Default::default()
        })
    }
    /// Select only granted native tools and pass every value as a literal argument to the owned job.
    fn begin(&mut self, call: &commands::Invocation) -> Result<(), Failure> {
        if call.id != "import" {
            return Err(invalid("Unknown Maven command"));
        }
        if self.pending.len() >= 8 {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                "Maven import queue is full",
            ));
        }
        let options: Options = serde_json::from_value(call.arguments.clone()).map_err(invalid)?;
        self.begin_options(options, Some(call.reply.clone()))
    }
    /// Native UI and typed commands share the same import transaction and model publication gate.
    fn begin_options(
        &mut self,
        options: Options,
        reply: Option<api::ResourceHandle>,
    ) -> Result<(), Failure> {
        if self.pending.len() >= 8 {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                "Maven import queue is full",
            ));
        }
        options.validate()?;
        self.observe()?;
        plugin_protocol::language_context::invalidate()?;
        self.intent = self.intent.saturating_add(1);
        self.stale = true;
        self.panel.status("正在导入 / Importing");
        let args = self.native_arguments(options.clone(), self.intent)?;
        let handle = start_native(&args)?;
        self.pending.insert(
            handle.resource,
            Import {
                process: handle,
                reply,
                bytes: Vec::new(),
                errors: Vec::new(),
                intent: self.intent,
                options,
            },
        );
        Ok(())
    }
    /// Build-file changes retire pending results and every previous borrowed language context.
    fn cancel_imports(&mut self, message: &str, code: ErrorCode) {
        self.intent = self.intent.saturating_add(1);
        self.stale = true;
        let _ = plugin_protocol::language_context::invalidate();
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = api::guest::close_resource(pending.process);
            if let Some(reply) = pending.reply {
                let _ = commands::reply(reply, Err(Failure::new(code, message)));
            }
        }
        self.panel.status(message);
    }
    /// Conservative input matching also catches deleted POMs and incomplete watcher streams.
    fn is_model_input(&self, path: &str) -> bool {
        let path = path.replace('\\', "/");
        path.split('/')
            .any(|part| part.eq_ignore_ascii_case(".mvn"))
            || path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("pom.xml"))
            || self
                .model
                .as_ref()
                .and_then(|model| model["request"]["pom"].as_str())
                .is_some_and(|pom| path.eq_ignore_ascii_case(pom))
    }
    /// Subscriptions and roots stay live together and are revoked by the ordinary instance teardown.
    fn observe(&mut self) -> Result<(), Failure> {
        if self.watch.is_some() {
            return Ok(());
        }
        let api::Value::Resource(root) = api::guest::request(api::Operation::OpenWorkspace)? else {
            return Err(invalid("Expected workspace observation root"));
        };
        match plugin_protocol::workspace::subscribe(root.clone()) {
            Ok(watch) => {
                self.watch = Some((root, watch));
                Ok(())
            }
            Err(error) => {
                let _ = api::guest::close_resource(root);
                Err(error)
            }
        }
    }
    /// Compose immutable native metadata using only this instance's host-validated tool choices.
    pub(crate) fn native_arguments(
        &self,
        options: Options,
        identity: u64,
    ) -> Result<Vec<String>, Failure> {
        let java = self.setting("maven.java");
        let java = process::inspect_tool(
            process::ToolSource::Program {
                program: if java.is_empty() { "java".into() } else { java },
            },
            0,
            vec![],
        )?;
        let context = process::native_context("maven")?;
        let args = vec![
            "--java".into(),
            java.program,
            "--workspace".into(),
            self.workspace.clone(),
            "--pom".into(),
            options.pom,
            "--data".into(),
            format!("{}/import-{}", context.data, identity),
            "--resources".into(),
            context.assets,
            "--local-home".into(),
            self.setting("maven.home"),
            "--repository".into(),
            self.setting("maven.repository"),
            "--profiles".into(),
            options.profiles.join(","),
            "--offline".into(),
            options.offline.to_string(),
        ];

        Ok(args)
    }
    /// Configuration values come only from the current host-validated effective setting map.
    fn setting(&self, key: &str) -> String {
        self.settings
            .get(key)
            .and_then(|entry| entry.value.as_str())
            .unwrap_or("")
            .into()
    }
    /// No successful model is published until the complete JSON channel and real process exit agree.
    fn process(&mut self, handle: &api::ResourceHandle, update: process::Update) {
        let Some(pending) = self
            .pending
            .get_mut(&handle.resource)
            .filter(|entry| entry.process == *handle)
        else {
            return;
        };
        let result = match update {
            process::Update::Output {
                stream: process::Stream::Stdout,
                bytes,
            } => {
                if pending.bytes.len() + bytes.len() <= 64 * 1024 {
                    pending.bytes.extend(bytes);
                    return;
                }
                let _ = api::guest::request(api::Operation::Process {
                    operation: process::Operation::Terminate {
                        handle: handle.clone(),
                    },
                });
                Err(Failure::new(
                    ErrorCode::LimitExceeded,
                    "Maven model exceeds the public response budget",
                ))
            }
            process::Update::Output { bytes, .. } => {
                pending.errors.extend(bytes);
                if pending.errors.len() > 8192 {
                    let tail = pending.errors.split_off(pending.errors.len() - 8192);
                    pending.errors = tail;
                }
                return;
            }
            process::Update::Exited { code: 0 } => {
                serde_json::from_slice::<Value>(&pending.bytes).map_err(invalid)
            }
            process::Update::Exited { code } => Err(Failure::new(
                ErrorCode::OperationFailed,
                format!(
                    "Maven import exited with code {code}: {}",
                    String::from_utf8_lossy(&pending.errors).trim()
                ),
            )),
            process::Update::Terminated => Err(Failure::new(
                ErrorCode::Cancelled,
                "Maven import was terminated",
            )),
        };
        let pending = self.pending.remove(&handle.resource).unwrap();
        let result = if pending.intent != self.intent {
            Err(Failure::new(
                ErrorCode::StaleRevision,
                "A newer Maven import superseded this result",
            ))
        } else {
            result.and_then(|mut report| {
                if report["schema"] != 1
                    || !report["modules"].is_array()
                    || !report["generation"].is_string()
                {
                    return Err(invalid("Invalid Maven model receipt"));
                }
                report["request"] = serde_json::to_value(&pending.options).map_err(invalid)?;
                let payload = serde_json::to_string(&report).map_err(invalid)?;
                // Keep room for the model-status wrapper and transport escaping before publication.
                if serde_json::to_vec(&json!({"payload":payload}))
                    .map_err(invalid)?
                    .len()
                    > 60 * 1024
                {
                    return Err(Failure::new(
                        ErrorCode::LimitExceeded,
                        "Maven model exceeds the public response budget",
                    ));
                }
                // Readonly requests may fail while this import is pending. Publishing fresh facts
                // starts another discovery epoch so those transient failures cannot remain cached.
                plugin_protocol::language_context::invalidate()?;
                self.model = Some(report);
                self.stale = false;
                Ok(json!({"payload":payload}))
            })
        };
        let _ = api::guest::close_resource(pending.process);
        match &result {
            Ok(_) => self.panel.status("导入成功 / Imported"),
            Err(error) => self.panel.status(error.message.clone()),
        };
        if let Some(reply) = pending.reply {
            let _ = commands::reply(reply, result);
        }
    }
}
/// Boundary errors retain a typed category rather than becoming partial or successful import receipts.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
/// Start the declared native service with bounded literal stdin metadata; retain host dependency leases.
pub(crate) fn start_native(args: &[String]) -> Result<api::ResourceHandle, Failure> {
    let api::Value::Resource(handle) = api::guest::request(api::Operation::Process {
        operation: process::Operation::StartService {
            service: "maven".into(),
        },
    })?
    else {
        return Err(invalid("Expected a Maven process resource"));
    };
    // The declared argv contains host-expanded dependency paths. Per-import metadata travels
    // through the owned stdin pipe, and cannot replace an executable or a dependency binding.
    let payload = serde_json::to_vec(args).map_err(invalid)?;
    let write = (|| {
        for bytes in payload.chunks(8192) {
            api::guest::request(api::Operation::Process {
                operation: process::Operation::Write {
                    handle: handle.clone(),
                    bytes: bytes.to_vec(),
                },
            })?;
        }
        api::guest::request(api::Operation::Process {
            operation: process::Operation::CloseStdin {
                handle: handle.clone(),
            },
        })?;
        Ok::<_, Failure>(())
    })();
    if let Err(error) = write {
        let _ = api::guest::close_resource(handle);
        return Err(error);
    }
    Ok(handle)
}
