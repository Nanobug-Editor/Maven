//! Preserve Maven's already mediated dependency order and scopes when projecting Java metadata.
use plugin_protocol::api::{ErrorCode, Failure};
use serde_json::{Value, json};

/// A bounded immutable projection never merges another reactor module's external dependency graph.
pub(crate) struct Project {
    pub(crate) java: Value,
    pub(crate) classpath: Vec<String>,
}

/// One reactor uses a shared analysis session with separate main/test projects and exact classpaths.
/// Libraries are pooled once so repeated reactor edges cannot exhaust the bounded metadata envelope.
pub(crate) fn analysis(modules: &[Value]) -> Result<(Value, String), Failure> {
    let modules: Vec<_> = modules
        .iter()
        .filter(|module| module["packaging"] != "pom")
        .collect();
    if modules.is_empty() || modules.len() > 64 {
        return Err(invalid("Invalid Java module count"));
    }
    let mut libraries: Vec<String> = Vec::new();
    let mut nodes = Vec::new();
    let mut jdk = String::new();
    for (index, module) in modules.iter().enumerate() {
        let level = module["jdk"]
            .as_str()
            .unwrap_or("")
            .strip_prefix("1.")
            .unwrap_or(module["jdk"].as_str().unwrap_or(""));
        if !level.is_empty() {
            let feature = level
                .parse::<u32>()
                .map_err(|_| invalid("Unresolved Java compiler level"))?;
            if feature > jdk.parse::<u32>().unwrap_or(0) {
                jdk = level.into();
            }
        }
        for test in [false, true] {
            let mut jars = Vec::new();
            let mut references = if test { vec![index * 2] } else { vec![] };
            for edge in module["dependencies"].as_array().into_iter().flatten() {
                let scope = edge["scope"].as_str().unwrap_or("compile");
                if !(matches!(scope, "compile" | "provided" | "system")
                    || test && matches!(scope, "runtime" | "test"))
                    || edge["type"] != "jar"
                {
                    continue;
                }
                if let Some(id) = edge["module"].as_str() {
                    let dependency = modules
                        .iter()
                        .position(|module| module["id"] == id)
                        .ok_or_else(|| invalid("Missing Java reactor module"))?;
                    let classifier = edge["classifier"].as_str().unwrap_or("");
                    if !matches!(classifier, "" | "tests") {
                        return Err(invalid("Unsupported reactor JAR classifier"));
                    }
                    let reference = dependency * 2 + usize::from(classifier == "tests");
                    if !references.contains(&reference) {
                        references.push(reference);
                    }
                } else {
                    let file = edge["file"]
                        .as_str()
                        .ok_or_else(|| invalid("Missing resolved Java library"))?;
                    let position = libraries
                        .iter()
                        .position(|path| path == file)
                        .unwrap_or_else(|| {
                            libraries.push(file.into());
                            libraries.len() - 1
                        });
                    if !jars.contains(&position) {
                        jars.push(position);
                    }
                }
            }
            nodes.push(json!({"sources":module[if test {"test_source_roots"} else {"source_roots"}],"libraries":jars,"references":references,"release":module["compiler"]["release"].as_str().unwrap_or(""),"source":module["compiler"]["source"].as_str().unwrap_or(""),"target":module["compiler"]["target"].as_str().unwrap_or("")}));
        }
    }
    Ok((
        json!({"schema":1,"project":{"source_roots":["."],"jars":[],"output":"out","main_class":""},"graph":{"libraries":libraries,"projects":nodes}}),
        jdk,
    ))
}

/// Project one selected module from the Resolver's complete effective closure. Reactor binaries may
/// be absent during import; source/output metadata replaces only those exact selected artifacts.
pub(crate) fn runtime_project(module: &Value, modules: &[Value]) -> Result<Project, Failure> {
    let output = module["output"]
        .as_str()
        .ok_or_else(|| invalid("Missing module output"))?;
    let mut sources = Vec::new();
    let mut jars = Vec::new();
    let mut classpath = vec![output.to_owned()];
    roots(module, "source_roots", &mut sources);
    for edge in module["dependencies"].as_array().into_iter().flatten() {
        let scope = edge["scope"].as_str().unwrap_or("compile");
        let visible = matches!(scope, "compile" | "runtime");
        if !visible || edge["type"] != "jar" {
            continue;
        }
        if let Some(id) = edge["module"].as_str() {
            let dependency = modules
                .iter()
                .find(|candidate| candidate["id"] == id)
                .ok_or_else(|| invalid("Missing reactor dependency"))?;
            let classifier = edge["classifier"].as_str().unwrap_or("");
            if !matches!(classifier, "" | "tests") {
                return Err(invalid(
                    "Unsupported reactor JAR classifier; use the Maven goal configuration",
                ));
            }
            let tests = classifier == "tests";
            roots(
                dependency,
                if tests {
                    "test_source_roots"
                } else {
                    "source_roots"
                },
                &mut sources,
            );
            push(
                &mut classpath,
                dependency[if tests { "test_output" } else { "output" }]
                    .as_str()
                    .ok_or_else(|| invalid("Missing reactor output"))?,
            );
        } else {
            let file = edge["file"]
                .as_str()
                .ok_or_else(|| invalid("Missing resolved JAR"))?;
            push(&mut jars, file);
            push(&mut classpath, file);
        }
    }
    Ok(Project {
        java: json!({"source_roots":sources,"jars":jars,"output":output,"main_class":""}),
        classpath,
    })
}

/// Source roots retain their first effective position, including generated roots supplied by Maven.
fn roots(module: &Value, field: &str, output: &mut Vec<String>) {
    for root in module[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        push(output, root);
    }
}
/// Classpath order resolves duplicate classes; lexical sorting must never replace Maven's order.
fn push(output: &mut Vec<String>, value: &str) {
    if !output.iter().any(|entry| entry == value) {
        output.push(value.into());
    }
}
/// Invalid imported facts fail explicitly before any native analysis or launch begins.
fn invalid(message: &str) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, message)
}
