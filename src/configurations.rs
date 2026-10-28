//! Maven owns portable POM/profile choices; the SDK composes ordinary native command fields.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    configurations::{self as contract, command_form::Fields},
    service, ui,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Catalog, form and validation are pure metadata operations; native execution is a separate target.
pub(crate) fn invoke(
    state: &crate::imports::State,
    call: service::Invocation,
) -> Result<api::Output, Failure> {
    if call.method == "validate" && call.arguments["template"] == "application" {
        return crate::application::validate(state, call);
    }
    let result = (|| {
        let locale = call.arguments["locale"].as_str().unwrap_or("zh-CN");
        let mut defaults = Values {
            pom: "pom.xml".into(),
            profiles: String::new(),
            offline: false,
            command: Fields::new(
                label(locale, "Maven 目标", "Maven Goal"),
                vec!["verify".into()],
            ),
            revision: 1,
            module: String::new(),
            main_class: String::new(),
        };
        let application = call.arguments["template"] == "application";
        if application {
            defaults.command = Fields::new(
                label(locale, "Maven Java 应用", "Maven Java Application"),
                vec![],
            );
            if let Ok((model, _)) = state.current_model() {
                if let Ok(options) =
                    serde_json::from_value::<crate::imports::Options>(model["request"].clone())
                {
                    defaults.pom = options.pom;
                    defaults.profiles = options.profiles.join(",");
                    defaults.offline = options.offline;
                }
                let modules = model["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|module| module["packaging"] != "pom")
                    .collect::<Vec<_>>();
                if modules.len() == 1 {
                    defaults.module = modules[0]["id"].as_str().unwrap_or("").into();
                }
            }
        }
        match call.method.as_str() {
            "catalog" => envelope(&contract::Catalog {
                templates: vec![
                    contract::Template {
                        id: "goal".into(),
                        group: "Maven".into(),
                        label: label(locale, "Maven 目标", "Maven Goal"),
                        icon: "build".into(),
                        defaults: serde_json::to_string(&defaults).map_err(invalid)?,
                        unavailable: None,
                    },
                    contract::Template {
                        id: "application".into(),
                        group: "Maven".into(),
                        label: label(locale, "Maven Java 应用", "Maven Java Application"),
                        icon: "play".into(),
                        defaults: serde_json::to_string(&Values {
                            command: Fields::new(
                                label(locale, "Maven Java 应用", "Maven Java Application"),
                                vec![],
                            ),
                            ..defaults
                        })
                        .map_err(invalid)?,
                        unavailable: None,
                    },
                ],
            }),
            "form" => {
                if call.arguments["template"] != "goal" && !application {
                    return Err(invalid("Unknown Maven template"));
                }
                let raw = call.arguments["values"].as_str().unwrap_or("");
                let mut values = if raw.is_empty() || raw == "{}" {
                    defaults
                } else {
                    serde_json::from_str::<Values>(raw).map_err(invalid)?
                };
                values.edit(call.arguments["event"].as_str().unwrap_or(""))?;
                if application {
                    values.select_module(state, call.arguments["event"].as_str().unwrap_or(""))?;
                }
                let mut document = values.command.document("maven", locale, values.revision);
                document.root = ui::Node::column(
                    "maven-form",
                    vec![
                        ui::Node::text(
                            "pom-label",
                            label(
                                locale,
                                "工程 POM（工作区相对路径）",
                                "Project POM (workspace relative)",
                            ),
                        ),
                        ui::Node::input(
                            "maven-pom",
                            ui::Input {
                                value: values.pom.clone(),
                                value_revision: values.command.input_revision,
                                placeholder: "pom.xml".into(),
                            },
                        ),
                        ui::Node::text(
                            "profiles-label",
                            label(locale, "Profiles（逗号分隔）", "Profiles (comma separated)"),
                        ),
                        ui::Node::input(
                            "maven-profiles",
                            ui::Input {
                                value: values.profiles.clone(),
                                value_revision: values.command.input_revision,
                                placeholder: "dev,local".into(),
                            },
                        ),
                        ui::Node::checkbox(
                            "maven-offline",
                            label(locale, "离线模式", "Offline mode"),
                            values.offline,
                        ),
                        document.root,
                    ],
                )
                .gap(6.)
                .grow();
                if application {
                    let mut nodes = vec![
                        ui::Node::text(
                            "application-module-label",
                            label(locale, "选择导入的模块", "Choose an imported module"),
                        ),
                        ui::Node::text("application-module", values.module.clone()),
                    ];
                    if let Ok((model, _)) = state.current_model() {
                        nodes.extend(
                            model["modules"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .enumerate()
                                .filter(|(_, module)| module["packaging"] != "pom")
                                .map(|(index, module)| {
                                    ui::Node::button(
                                        format!("maven-run-module-{index}"),
                                        module["id"].as_str().unwrap_or(""),
                                    )
                                }),
                        );
                    }
                    nodes.push(ui::Node::text(
                        "application-main-label",
                        label(locale, "Java 主类", "Java main class"),
                    ));
                    nodes.push(ui::Node::input(
                        "maven-main-class",
                        ui::Input {
                            value: values.main_class.clone(),
                            value_revision: values.command.input_revision,
                            placeholder: "com.example.Main".into(),
                        },
                    ));
                    nodes.push(document.root);
                    document.root = ui::Node::column("maven-application-form", nodes)
                        .gap(6.)
                        .grow();
                }
                envelope(&contract::Form {
                    values: serde_json::to_string(&values).map_err(invalid)?,
                    name: values.command.name.clone(),
                    program: "Maven".into(),
                    document,
                })
            }
            "validate" => {
                let values: Values =
                    serde_json::from_str(call.arguments["values"].as_str().unwrap_or(""))
                        .map_err(invalid)?;
                let options = values.options();
                let problem = options
                    .validate()
                    .err()
                    .map(|failure| failure.message)
                    .or_else(|| values.command.problem(locale));
                if let Some(message) = problem {
                    return envelope(&contract::Validation {
                        valid: false,
                        message,
                        launch: None,
                    });
                }
                let mut launch = values
                    .command
                    .launch("maven", call.arguments["workspace"].as_str().unwrap_or(""));
                launch.target = json!({"mode":"provided","provider":"maven","label":values.command.name,
                    "binding":json!({"schema":1,"pom":options.pom,"profiles":options.profiles,"offline":options.offline}).to_string(),"args":values.command.args});
                envelope(&contract::Validation {
                    valid: true,
                    message: String::new(),
                    launch: Some(launch),
                })
            }
            _ => Err(invalid("Unknown Maven configuration method")),
        }
    })();
    Ok(api::Output {
        service_reply: Some(result),
        ..Default::default()
    })
}
/// Drafts round-trip through the host's normal configuration store, including literal argv/env rows.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Values {
    pub(crate) pom: String,
    pub(crate) profiles: String,
    pub(crate) offline: bool,
    pub(crate) command: Fields,
    revision: u64,
    #[serde(default)]
    pub(crate) module: String,
    #[serde(default)]
    pub(crate) main_class: String,
}
impl Values {
    /// Convert the profile editor into bounded Maven tokens, without shell parsing or interpolation.
    pub(crate) fn options(&self) -> crate::imports::Options {
        crate::imports::Options {
            pom: self.pom.clone(),
            profiles: self
                .profiles
                .split(',')
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_owned)
                .collect(),
            offline: self.offline,
        }
    }
    /// Native module buttons refer only to the current model, never a persisted array position.
    fn select_module(&mut self, state: &crate::imports::State, event: &str) -> Result<(), Failure> {
        if event.is_empty() {
            return Ok(());
        }
        if let contract::FormEvent::Native(ui::UiEvent {
            node,
            action: ui::Action::Click,
            ..
        }) = serde_json::from_str(event).map_err(invalid)?
            && let Some(index) = node.strip_prefix("maven-run-module-")
        {
            let index = index.parse::<usize>().map_err(invalid)?;
            let (model, _) = state.current_model()?;
            self.module = model["modules"]
                .get(index)
                .filter(|module| module["packaging"] != "pom")
                .and_then(|module| module["id"].as_str())
                .ok_or_else(|| invalid("The Maven module choice has expired"))?
                .into();
        }
        Ok(())
    }
    /// Maven-specific fields retain IME revisions; ordinary command edits use the SDK's native rows.
    fn edit(&mut self, event: &str) -> Result<(), Failure> {
        if event.is_empty() {
            return Ok(());
        }
        match serde_json::from_str::<contract::FormEvent>(event).map_err(invalid)? {
            contract::FormEvent::Native(ui::UiEvent {
                node,
                action: ui::Action::Change(value) | ui::Action::Submit(value),
                ..
            }) if node == "maven-main-class" => self.main_class = value,
            contract::FormEvent::Native(ui::UiEvent {
                node,
                action: ui::Action::Change(value) | ui::Action::Submit(value),
                ..
            }) if node == "maven-pom" => self.pom = value,
            contract::FormEvent::Native(ui::UiEvent {
                node,
                action: ui::Action::Change(value) | ui::Action::Submit(value),
                ..
            }) if node == "maven-profiles" => self.profiles = value,
            contract::FormEvent::Native(ui::UiEvent {
                node,
                action: ui::Action::Toggle(value),
                ..
            }) if node == "maven-offline" => self.offline = value,
            _ => self.command.edit(event)?,
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Maven form revision exhausted"))?;
        Ok(())
    }
}
/// Public service envelopes stay bounded by the canonical configuration declaration.
fn envelope(value: &impl Serialize) -> Result<Value, Failure> {
    Ok(json!({"payload":serde_json::to_string(value).map_err(invalid)?}))
}
/// Invalid drafts produce explicit errors without modifying the saved configuration.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
/// Provider labels use the native host's locale; user-entered paths and arguments remain literal.
fn label(locale: &str, zh: &str, en: &str) -> String {
    if locale.starts_with("zh") { zh } else { en }.into()
}
