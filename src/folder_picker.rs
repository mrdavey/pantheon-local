use anyhow::{Context, Result, bail, ensure};
use std::{
    path::Path,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use wait_timeout::ChildExt;

#[cfg(target_os = "macos")]
fn picker() -> Result<Command> {
    let mut command = Command::new("/usr/bin/osascript");
    command.args([
        "-e",
        r#"try
    activate
    return POSIX path of (choose folder with prompt "Choose a project folder")
on error number -128
    return ""
end try"#,
    ]);
    Ok(command)
}

#[cfg(not(target_os = "macos"))]
fn picker() -> Result<Command> {
    let (program, args): (&str, &[&str]) = if crate::adapter::installed("zenity") {
        (
            "zenity",
            &[
                "--file-selection",
                "--directory",
                "--title=Choose a project folder",
            ],
        )
    } else if crate::adapter::installed("kdialog") {
        (
            "kdialog",
            &[
                "--getexistingdirectory",
                ".",
                "--title",
                "Choose a project folder",
            ],
        )
    } else {
        bail!("Enter the project path, or install Zenity or KDialog to browse folders.")
    };
    let mut command = Command::new(program);
    command.args(args);
    Ok(command)
}

fn run_picker(mut command: Command, shutdown: &AtomicBool) -> Result<Option<Output>> {
    use std::io::{Read, Seek};
    let mut capture = tempfile::tempfile()?;
    let errors = tempfile::tempfile()?;
    let mut child = command
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(capture.try_clone()?)
        .stderr(errors.try_clone()?)
        .spawn()
        .context("Could not open the system picker.")?;
    let status = loop {
        if let Some(status) = child.wait_timeout(Duration::from_millis(250))? {
            break status;
        }
        if shutdown.load(Ordering::Acquire) {
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
    };
    #[cfg(not(target_os = "macos"))]
    if status.code() == Some(1) && errors.metadata()?.len() == 0 {
        return Ok(None);
    }
    ensure!(
        status.success(),
        "Could not open the system picker. Check the desktop session."
    );
    capture.rewind()?;
    let mut stdout = Vec::new();
    capture.read_to_end(&mut stdout)?;
    Ok(Some(Output {
        status,
        stdout,
        stderr: Vec::new(),
    }))
}

pub fn choose(shutdown: &AtomicBool) -> Result<Option<String>> {
    Ok(run_picker(picker()?, shutdown)?
        .map(selected)
        .transpose()?
        .flatten())
}

#[cfg(target_os = "macos")]
fn file_paths(root: &str, shutdown: &AtomicBool) -> Result<Vec<String>> {
    // Pass the folder as argv. JSON preserves whitespace and newlines in names.
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-l", "JavaScript", "-e", r#"function run(argv) {
        const app = Application.currentApplication();
        app.includeStandardAdditions = true;
        app.activate();
        try {
            const files = app.chooseFile({withPrompt: "Choose initial project files", defaultLocation: Path(argv[0]), multipleSelectionsAllowed: true});
            return JSON.stringify(files.map(file => file.toString()));
        } catch (error) {
            if (error.errorNumber === -128) return "[]";
            throw error;
        }
    }"#, "--", root]);
    match run_picker(command, shutdown)? {
        Some(out) => Ok(serde_json::from_slice(&out.stdout)?),
        None => Ok(Vec::new()),
    }
}

#[cfg(not(target_os = "macos"))]
fn file_paths(root: &str, shutdown: &AtomicBool) -> Result<Vec<String>> {
    let zenity = crate::adapter::installed("zenity");
    let separator = if zenity { crate::uid() } else { "\n".into() };
    let mut command = if zenity {
        let mut command = Command::new("zenity");
        command
            .args([
                "--file-selection",
                "--multiple",
                "--title=Choose initial project files",
            ])
            .arg(format!("--filename={root}/"))
            .arg(format!("--separator={separator}"));
        command
    } else {
        ensure!(
            crate::adapter::installed("kdialog"),
            "Install Zenity or KDialog to choose initial files."
        );
        let mut command = Command::new("kdialog");
        command.args([
            "--getopenurl",
            root,
            "--multiple",
            "--separate-output",
            "--title",
            "Choose initial project files",
        ]);
        command
    };
    command.env_remove("PWD");
    let Some(output) = run_picker(command, shutdown)? else {
        return Ok(vec![]);
    };
    let raw = String::from_utf8(output.stdout)?;
    let raw = raw.strip_suffix('\n').unwrap_or(&raw);
    if raw.is_empty() {
        return Ok(vec![]);
    }
    raw.split(&separator)
        .map(|entry| {
            if zenity {
                return Ok(entry.to_owned());
            }
            let file = url::Url::parse(entry)?
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("Choose a local file."))?;
            file.into_os_string()
                .into_string()
                .map_err(|_| anyhow::anyhow!("File names must be valid UTF-8."))
        })
        .collect()
}

pub fn choose_initial(project: &str, shutdown: &AtomicBool) -> Result<Vec<String>> {
    let choices = crate::project::initial_choices(project)?;
    let root = choices["root"].as_str().context("Missing Git root")?;
    let chosen = file_paths(root, shutdown)?;
    relative_choices(root, &choices["files"], &chosen)
}

fn relative_choices(
    root: &str,
    available: &serde_json::Value,
    chosen: &[String],
) -> Result<Vec<String>> {
    let allowed: std::collections::HashSet<_> = available
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    let mut files = Vec::new();
    for path in chosen {
        let selected = Path::new(path);
        // Finder may return /var or /tmp for a project opened via /private.
        // Resolve parent aliases while preserving a selected symlink's Git name.
        let resolved = selected
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .zip(selected.file_name())
            .map(|(parent, name)| parent.join(name));
        let relative = selected
            .strip_prefix(root)
            .ok()
            .or_else(|| resolved.as_deref()?.strip_prefix(root).ok())
            .and_then(Path::to_str)
            .filter(|p| allowed.contains(*p))
            .context("Choose nonignored files inside the project folder.")?;
        if !files.iter().any(|p| p == relative) {
            files.push(relative.to_owned());
        }
    }
    Ok(files)
}

fn selected(output: Output) -> Result<Option<String>> {
    if !output.status.success() {
        bail!("Could not open the folder picker. Enter the project path instead.")
    }
    let raw = String::from_utf8(output.stdout)?;
    let path = raw.strip_suffix('\n').unwrap_or(&raw);
    if path.is_empty() {
        return Ok(None);
    }
    ensure!(
        Path::new(path).is_absolute() && Path::new(path).is_dir(),
        "Choose an accessible project folder."
    );
    Ok(Some(path.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::process::ExitStatusExt, process::ExitStatus};

    #[test]
    fn native_multiple_selection_preserves_paths_and_checks_git_membership() -> Result<()> {
        let choices = serde_json::json!(["a/x", "b/x"]);
        assert_eq!(
            relative_choices(
                "/project",
                &choices,
                &[
                    "/project/b/x".into(),
                    "/project/a/x".into(),
                    "/project/a/x".into()
                ]
            )?,
            ["b/x", "a/x"]
        );
        assert!(relative_choices("/project", &choices, &["/other/a/x".into()]).is_err());
        assert!(relative_choices("/project", &choices, &["/project/ignored".into()]).is_err());
        Ok(())
    }
    #[test]
    fn selection_preserves_folder_names_and_cancel_preserves_an_empty_result() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let folder = tmp.path().join("résumé folder ");
        fs::create_dir(&folder)?;
        let reply = |s: &str| Output {
            status: ExitStatus::from_raw(0),
            stdout: format!("{s}\n").into(),
            stderr: vec![],
        };
        assert_eq!(
            selected(reply(folder.to_str().unwrap()))?,
            Some(folder.to_string_lossy().into_owned())
        );
        assert_eq!(selected(reply(""))?, None);
        assert!(selected(reply("relative")).is_err());
        Ok(())
    }
}
