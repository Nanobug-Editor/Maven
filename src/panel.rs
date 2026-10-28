//! One native project view owns only import choices and presentation, never editor text or tool state.
use plugin_protocol::{
    api::{self, ErrorCode, Failure},
    ui,
};
use serde_json::Value;

/// Draft paths and profiles are independent from the last successfully imported immutable model.
pub(crate) struct ProjectPanel {
    pom: String,
    profiles: String,
    offline: bool,
    message: String,
    candidates: Vec<String>,
    revision: u64,
    input_revision: u64,
}
impl Default for ProjectPanel {
    /// Explicit import is required before analysis; merely opening this view starts no Maven process.
    fn default() -> Self {
        Self {
            pom: "pom.xml".into(),
            profiles: String::new(),
            offline: false,
            message: "尚未导入 / Not imported".into(),
            candidates: vec![],
            revision: 1,
            input_revision: 0,
        }
    }
}
impl ProjectPanel {
    /// Decode native revisions/controls after the host's ordinary view admission; return only import intent.
    pub(crate) fn edit(
        &mut self,
        event: ui::UiEvent,
    ) -> Result<Option<crate::imports::Options>, Failure> {
        match (event.node.as_str(), event.action) {
            ("maven-pom", ui::Action::Change(value) | ui::Action::Submit(value)) => {
                self.pom = value
            }
            ("maven-profiles", ui::Action::Change(value) | ui::Action::Submit(value)) => {
                self.profiles = value
            }
            ("maven-offline", ui::Action::Toggle(value)) => self.offline = value,
            ("maven-import", ui::Action::Click) => {
                return Ok(Some(crate::imports::Options {
                    pom: self.pom.clone(),
                    profiles: self
                        .profiles
                        .split(',')
                        .map(str::trim)
                        .filter(|profile| !profile.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    offline: self.offline,
                }));
            }
            ("maven-find", ui::Action::Click) => self.discover()?,
            (node, ui::Action::Click) if node.starts_with("maven-choice-") => {
                let index = node
                    .trim_start_matches("maven-choice-")
                    .parse::<usize>()
                    .map_err(invalid)?;
                self.pom = self
                    .candidates
                    .get(index)
                    .ok_or_else(|| invalid("Maven POM choice has expired"))?
                    .clone();
                self.candidates.clear();
                self.input_revision = self.input_revision.saturating_add(1);
            }
            _ => {}
        }
        self.revision = self.revision.saturating_add(1);
        Ok(None)
    }
    /// Filesystem discovery returns bounded relative POM choices through the public workspace handle.
    fn discover(&mut self) -> Result<(), Failure> {
        let api::Value::Resource(root) = api::guest::request(api::Operation::OpenWorkspace)? else {
            return Err(invalid("Expected a workspace handle"));
        };
        let found = api::guest::request(api::Operation::FindFiles {
            handle: root.clone(),
            query: api::FileQuery {
                include: vec!["pom.xml".into(), "**/pom.xml".into()],
                exclude: vec!["**/target/**".into(), "**/.git/**".into()],
                max_results: 128,
            },
        });
        let _ = api::guest::close_resource(root);
        let api::Value::Files(found) = found? else {
            return Err(invalid("Expected Maven POM candidates"));
        };
        self.candidates = found.paths;
        self.candidates.sort();
        self.candidates.dedup();
        self.message = format!(
            "发现 {} 个 POM / {} POM candidates{}",
            self.candidates.len(),
            self.candidates.len(),
            if found.skipped.is_empty() {
                ""
            } else {
                "；部分目录不可读 / some directories were unreadable"
            }
        );
        Ok(())
    }
    /// State transitions advance the native document revision; input composition keeps its own revision.
    pub(crate) fn status(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.revision = self.revision.saturating_add(1);
    }
    /// The ordinary native host owns rendering, focus, IME, keyboard, theme and scroll behavior.
    pub(crate) fn view(&self, model: Option<&Value>, stale: bool, importing: bool) -> api::View {
        let mut nodes = vec![
            ui::Node::text("maven-title", "Maven 工程 / Maven Project"),
            ui::Node::text(
                "maven-pom-label",
                "POM（工作区相对路径 / workspace relative）",
            ),
            ui::Node::input(
                "maven-pom",
                ui::Input {
                    value: self.pom.clone(),
                    value_revision: self.input_revision,
                    placeholder: "pom.xml".into(),
                },
            ),
            ui::Node::button("maven-find", "发现 POM / Find POMs"),
        ];
        nodes.extend(
            self.candidates
                .iter()
                .enumerate()
                .map(|(index, path)| ui::Node::button(format!("maven-choice-{index}"), path)),
        );
        nodes.extend([
            ui::Node::text(
                "maven-profiles-label",
                "Profiles（逗号分隔 / comma separated）",
            ),
            ui::Node::input(
                "maven-profiles",
                ui::Input {
                    value: self.profiles.clone(),
                    value_revision: self.input_revision,
                    placeholder: "dev,local".into(),
                },
            ),
            ui::Node::checkbox("maven-offline", "离线 / Offline", self.offline),
            ui::Node::button(
                "maven-import",
                if importing {
                    "正在导入 / Importing"
                } else {
                    "导入／刷新 / Import / Refresh"
                },
            ),
            ui::Node::text("maven-status", self.message.clone()),
        ]);
        if importing {
            nodes.push(ui::Node::button("maven-cancel", "取消导入 / Cancel import"));
        }
        if stale && model.is_some() {
            nodes.push(ui::Node::text(
                "maven-stale",
                "模型已过期，请刷新 / Model is stale; refresh required",
            ));
        }
        if let Some(model) = model {
            nodes.extend(
                model["modules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(index, module)| {
                        ui::Node::text(
                            format!("maven-module-{index}"),
                            format!(
                                "{}  ·  Java {}",
                                module["id"].as_str().unwrap_or(""),
                                module["jdk"].as_str().unwrap_or("")
                            ),
                        )
                    }),
            );
        }
        api::View {
            panel: "project".into(),
            document: ui::Document {
                revision: self.revision,
                ..ui::Document::new(ui::Node::column("maven-project", nodes).gap(6.).grow())
            },
        }
    }
}
/// Native metadata errors remain explicit and do not reset the user's current import choices.
fn invalid(error: impl ToString) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, error.to_string())
}
