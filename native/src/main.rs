//! A controlled native entry pins the workspace and invokes JVM/Maven with literal arguments.
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Report native setup failures on stderr; stdout belongs exclusively to the imported model.
fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Maven: {error}");
            std::process::exit(1);
        }
    }
}
/// Arguments are paired metadata from the owned guest; no command string is interpreted by a shell.
fn run() -> Result<i32, Box<dyn Error>> {
    let mut values = BTreeMap::new();
    let mut direct = std::env::args().skip(1).collect::<Vec<_>>();
    let goals = if let Some(index) = direct.iter().position(|argument| argument == "--") {
        let tail = direct.split_off(index + 1);
        direct.pop();
        tail
    } else {
        vec![]
    };
    let mut data = Vec::new();
    if let Some(index) = direct.iter().position(|argument| argument == "--metadata") {
        let encoded = direct
            .get(index + 1)
            .ok_or("Missing Maven metadata")?
            .as_bytes()
            .to_vec();
        direct.drain(index..=index + 1);
        data = encoded;
    } else {
        std::io::stdin().take(65537).read_to_end(&mut data)?;
    }
    if data.len() > 65536 {
        return Err("Maven metadata input quota exceeded".into());
    }
    let metadata: Vec<String> = serde_json::from_slice(&data)?;
    let mut input = direct.into_iter().chain(metadata);
    while let Some(key) = input.next() {
        let value = input.next().ok_or("Missing Maven option value")?;
        if !key.starts_with("--") || values.insert(key, value).is_some() {
            return Err("Invalid Maven options".into());
        }
    }
    let value = |key: &str| {
        values
            .get(key)
            .map(String::as_str)
            .ok_or("Missing Maven native option")
    };
    let workspace = PathBuf::from(value("--workspace")?).canonicalize()?;
    let relative = Path::new(value("--pom")?);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("POM must stay inside the workspace".into());
    }
    let pom = workspace.join(relative).canonicalize()?;
    if !pom.starts_with(&workspace) || !pom.is_file() {
        return Err("POM escapes the workspace".into());
    }
    let mut directories = BTreeSet::new();
    for path in [&workspace, pom.parent().unwrap()] {
        directories.extend(path.ancestors().map(Path::to_owned));
    }
    // Deny ancestor renames through child exit; no second mutable source or project copy is created.
    let _pins = directories
        .iter()
        .map(|path| pin_directory(path))
        .collect::<std::io::Result<Vec<_>>>()?;
    let home = if value("--local-home")?.is_empty() {
        PathBuf::from(value("--home")?)
    } else {
        PathBuf::from(value("--local-home")?)
    }
    .canonicalize()?;
    if !home.join("lib").is_dir() || !home.join("boot").is_dir() {
        return Err("Invalid explicit Maven home".into());
    }
    let mode = values.get("--mode").map(String::as_str).unwrap_or("import");
    // Goal sessions inherit the host-validated configuration directory. Import always uses its
    // workspace, so a user's current terminal directory cannot change model discovery.
    let cwd = if mode == "goal" {
        std::env::current_dir()?.canonicalize()?
    } else {
        workspace.clone()
    };
    if mode == "describe" {
        println!("{}", serde_json::json!({"home":home.display().to_string()}));
        return Ok(0);
    }
    if !matches!(mode, "import" | "goal") {
        return Err("Invalid Maven execution mode".into());
    }
    let resources = PathBuf::from(value("--resources")?).canonicalize()?;
    let bridge = resources.join("native/maven-bridge.jar").canonicalize()?;
    if !bridge.starts_with(&resources) {
        return Err("Maven bridge escapes its package".into());
    }
    let java = PathBuf::from(value("--java")?).canonicalize()?;
    // Maven's launcher starts its .mvn search at the explicit POM. Keep independent nested
    // projects' CLI configuration separate while remaining within this authorized workspace.
    let base = pom
        .parent()
        .unwrap()
        .ancestors()
        .take_while(|path| path.starts_with(&workspace))
        .find(|path| path.join(".mvn").is_dir())
        .unwrap_or_else(|| pom.parent().unwrap());
    let classpath = std::env::join_paths([
        java_path(&bridge),
        java_path(&home.join("lib/*")),
        java_path(&home.join("boot/*")),
    ])?;
    let mut command = Command::new(&java);
    command
        .args([
            "-Dfile.encoding=UTF-8",
            "-Dsun.stdout.encoding=UTF-8",
            "-Dsun.stderr.encoding=UTF-8",
        ])
        .arg(format!("-Dmaven.home={}", java_path(&home).display()))
        .arg(format!(
            "-Dmaven.conf={}",
            java_path(&home.join("conf")).display()
        ))
        .arg(format!(
            "-Dmaven.multiModuleProjectDirectory={}",
            java_path(base).display()
        ))
        .arg(format!(
            "-Dmaven.ext.class.path={}",
            java_path(&bridge).display()
        ))
        .arg("-cp")
        .arg(classpath)
        .arg("org.nanobug.maven.Bridge")
        .arg(java_path(&workspace))
        .arg(java_path(&pom))
        .arg(value("--data")?)
        .arg(value("--repository")?)
        .arg(value("--profiles")?)
        .arg(value("--offline")?);
    command.arg(java_path(&cwd));
    if mode == "goal" {
        if goals.is_empty() {
            return Err("Maven requires at least one goal".into());
        }
        command.arg("goal").args(goals);
        #[cfg(windows)]
        unsafe {
            // Only this execution's inherited native console changes encoding before the JVM starts.
            windows_sys::Win32::System::Console::SetConsoleCP(65001);
            windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);
        }
    }
    command
        .current_dir(&cwd)
        .stdin(if mode == "goal" {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    if mode != "goal" {
        use std::os::windows::process::CommandExt;
        // Pipe services have no console. Goal sessions inherit their owned ConPTY and must keep
        // that console attached so the JVM's inherited console handles remain usable.
        command.creation_flags(0x08000000);
    }
    Ok(command.status()?.code().unwrap_or(1))
}
/// Checked paths use ordinary DOS/UNC notation for older Java NIO implementations; identity stays pinned.
fn java_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else {
        PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    }
}
/// Windows handles allow file changes but prevent replacing checked directory ancestors while Maven runs.
fn pin_directory(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000).share_mode(3);
    }
    options.open(path)
}
