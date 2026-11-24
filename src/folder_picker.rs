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

pub fn choose(shutdown: &AtomicBool) -> Result<Option<String>> {
    let mut child = picker()?
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not open the folder picker. Enter the project path instead.")?;
    while child.wait_timeout(Duration::from_millis(250))?.is_none() {
        if shutdown.load(Ordering::Acquire) {
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
    }
    let output = child.wait_with_output()?;
    #[cfg(not(target_os = "macos"))]
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    selected(output)
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
