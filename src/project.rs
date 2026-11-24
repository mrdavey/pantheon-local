use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{Read, Seek},
    path::{Component, Path},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Repository {
    pub root: String,
    pub common: String,
    pub subdir: String,
    pub head: String,
    pub target: String,
}

fn git_command(dir: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    for (key, _) in env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("LC_ALL", "C");
    cmd
}

fn folder_access_error(path: &Path) -> anyhow::Error {
    let help = if cfg!(target_os = "macos") {
        "Allow the launching terminal to access this folder in System Settings → Privacy & Security → Files & Folders, or choose another folder."
    } else {
        "Check the folder permissions or choose another folder."
    };
    anyhow::anyhow!("Cannot access {}. {help}", path.display())
}

fn output(cmd: &mut Command) -> Result<String> {
    let cwd = cmd.get_current_dir().unwrap_or(Path::new(".")).to_owned();
    let out = cmd.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            folder_access_error(&cwd)
        } else {
            anyhow::anyhow!("Could not start Git in {}: {err}", cwd.display())
        }
    })?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success()
        && stderr.contains("current working directory")
        && (stderr.contains("Operation not permitted") || stderr.contains("Permission denied"))
    {
        return Err(folder_access_error(&cwd));
    }
    ensure!(
        out.status.success(),
        "Git failed ({}): {}{}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim_end(),
        String::from_utf8_lossy(&out.stdout).trim_end()
    );
    let mut value =
        String::from_utf8(out.stdout).context("Git returned a non-UTF-8 path or response")?;
    if value.ends_with('\n') {
        value.pop();
    }
    Ok(value)
}

pub fn git(dir: &str, args: &[&str]) -> Result<String> {
    output(git_command(dir).args(args))
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("project path must be valid UTF-8")
}

pub fn discover(path: &str) -> Result<Repository> {
    let path = fs::canonicalize(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            folder_access_error(Path::new(path))
        } else {
            anyhow::anyhow!("Cannot open project folder {path}: {err}")
        }
    })?;
    fs::read_dir(&path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            folder_access_error(&path)
        } else {
            anyhow::anyhow!("Cannot read project folder {}: {err}", path.display())
        }
    })?;
    let root = git(path_text(&path)?, &["rev-parse", "--show-toplevel"])?;
    let common = git(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let target = git(&root, &["symbolic-ref", "--quiet", "HEAD"]).unwrap_or_default();
    let head = match head(&root) {
        Ok(v) => v,
        Err(e) if !target.is_empty() && e.to_string().contains("Needed a single revision") => {
            String::new()
        }
        Err(e) => return Err(e),
    };
    let subdir = path_text(path.strip_prefix(&root)?)?;
    Ok(Repository {
        root,
        common,
        head,
        target,
        subdir: if subdir.is_empty() { "." } else { subdir }.into(),
    })
}

fn check_initial_files(mut git: Command, root: &str, selected: &Path) -> Result<()> {
    let entries = git
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ])
        .output()?;
    ensure!(
        entries.status.success(),
        "Cannot check initial files in {root}: {}",
        String::from_utf8_lossy(&entries.stderr).trim()
    );
    if !entries.stdout.is_empty() {
        let location = if Path::new(root) == selected {
            format!("{root} contains files and has no commits.")
        } else {
            format!(
                "{} belongs to the Git repository at {root}, which contains files and has no commits.",
                selected.display()
            )
        };
        bail!(
            "{location} Choose a separate empty project folder, select files in Advanced settings → Initial import files (CLI: --import), or commit them in Git."
        );
    }
    Ok(())
}

/// List the same untracked, nonignored files Git will accept for the first commit.
pub fn initial_choices(path: &str) -> Result<serde_json::Value> {
    let selected =
        fs::canonicalize(path).with_context(|| format!("Cannot open project folder {path}"))?;
    let temp;
    let (root, mut cmd) = match discover(path) {
        Ok(repo) => {
            ensure!(
                repo.head.is_empty(),
                "This project already has a commit. Workers start from the committed files."
            );
            ensure!(
                git(&repo.root, &["ls-files", "-z"])?.is_empty(),
                "Create the initial commit in Git to preserve your staged work."
            );
            let cmd = git_command(&repo.root);
            (repo.root, cmd)
        }
        Err(err) if err.to_string().contains("not a git repository") => {
            let root = path_text(&selected)?;
            temp = tempfile::tempdir()?;
            git(
                root,
                &["init", "--bare", "--quiet", path_text(temp.path())?],
            )?;
            let mut cmd = git_command(root);
            cmd.arg("--git-dir")
                .arg(temp.path())
                .arg("--work-tree")
                .arg(&selected);
            (root.to_owned(), cmd)
        }
        Err(err) => return Err(err),
    };
    let names = output(cmd.args(["ls-files", "--others", "--exclude-standard", "-z"]))?;
    let files: Vec<_> = names
        .split_terminator('\0')
        .filter(|name| !name.ends_with('/'))
        .collect();
    Ok(serde_json::json!({"root": root, "files": files}))
}

pub fn open(path: &str, files: &[String], initialize: bool) -> Result<Repository> {
    let repo = match discover(path) {
        Ok(repo) => repo,
        Err(err) if initialize && err.to_string().contains("not a git repository") => {
            let canonical = fs::canonicalize(path)?;
            for ancestor in canonical.ancestors() {
                match fs::symlink_metadata(ancestor.join(".git")) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    _ => bail!(
                        "cannot open enclosing Git metadata in {}",
                        ancestor.display()
                    ),
                }
            }
            let root = path_text(&canonical)?;
            // Check Git's ignore rules before creating a repository in the user's folder.
            if files.is_empty() && fs::read_dir(&canonical)?.next().transpose()?.is_some() {
                let probe = tempfile::tempdir()?;
                git(
                    root,
                    &["init", "--bare", "--quiet", path_text(probe.path())?],
                )?;
                let mut cmd = git_command(root);
                cmd.arg("--git-dir")
                    .arg(probe.path())
                    .arg("--work-tree")
                    .arg(&canonical);
                check_initial_files(cmd, root, &canonical)?;
            }
            git(root, &["init", "--quiet"])?;
            discover(path)?
        }
        Err(e) => return Err(e),
    };
    if !repo.head.is_empty() {
        ensure!(
            files.is_empty(),
            "initial files are only accepted on an unborn branch"
        );
        return Ok(repo);
    }
    ensure!(initialize, "initialize or commit a base before starting");
    ensure!(
        git(&repo.root, &["ls-files", "-z"])?.is_empty(),
        "create the initial commit in Git to preserve your staged work"
    );
    if files.is_empty() {
        check_initial_files(
            git_command(&repo.root),
            &repo.root,
            &fs::canonicalize(path)?,
        )?;
    }
    let scratch = tempfile::tempdir()?;
    let indexed = |args: &[&str]| {
        output(
            git_command(&repo.root)
                .env("GIT_INDEX_FILE", scratch.path().join("index"))
                .args(args),
        )
    };
    for file in files {
        let local = Path::new(file);
        ensure!(
            !file.is_empty()
                && local
                    .components()
                    .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
            "initial import requires a repo-relative file: {file}"
        );
        let info = fs::symlink_metadata(Path::new(&repo.root).join(local))?;
        ensure!(
            info.is_file() || info.is_symlink(),
            "initial import requires a file"
        );
    }
    if !files.is_empty() {
        let mut args = vec!["--literal-pathspecs", "add", "--"];
        args.extend(files.iter().map(String::as_str));
        indexed(&args)?;
    }
    indexed(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "Establish the project base",
    ])?;
    if !files.is_empty() {
        let mut args = vec!["--literal-pathspecs", "reset", "--quiet", "HEAD", "--"];
        args.extend(files.iter().map(String::as_str));
        git(&repo.root, &args)?;
    }
    discover(path)
}

pub fn clone_repo(source: &str, dir: &str, base: &str, branch: &str) -> Result<()> {
    let parent = Path::new(dir).parent().context("missing clone parent")?;
    fs::create_dir_all(parent)?;
    git(
        path_text(parent)?,
        &[
            "clone",
            "--no-local",
            "--no-checkout",
            "--quiet",
            "--",
            source,
            dir,
        ],
    )?;
    fetch(dir, source, base)?;
    git(dir, &["checkout", "--quiet", "-B", branch, base])?;
    git(dir, &["remote", "remove", "origin"])?;
    Ok(())
}
pub fn apply_commit_policy(source: &str, target: &str) -> Result<()> {
    for key in [
        "user.name",
        "user.email",
        "user.signingkey",
        "commit.gpgsign",
        "gpg.format",
        "gpg.program",
        "gpg.ssh.program",
    ] {
        if let Ok(val) = git(source, &["config", "--get", key]) {
            git(target, &["config", "--local", key, &val])?;
        }
    }
    let hooks = git(
        source,
        &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    )?;
    git(target, &["config", "--local", "core.hooksPath", &hooks])?;
    Ok(())
}
pub fn head(dir: &str) -> Result<String> {
    git(dir, &["rev-parse", "--verify", "HEAD^{commit}"])
}
pub fn status(dir: &str) -> Result<String> {
    git(
        dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ],
    )
}
pub fn is_ancestor(dir: &str, base: &str, sha: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", base, sha]).is_ok()
}
/// Record only the worker's explicit handoff, leaving unrelated index entries alone.
pub fn commit_handoff(
    dir: &str,
    base: &str,
    task: &str,
    turn: &str,
    files: &[String],
    scopes: &[String],
    summary: &str,
) -> Result<String> {
    ensure!(
        !files.is_empty() && !summary.trim().is_empty(),
        "a ready milestone needs reviewed files and a validation summary"
    );
    for file in files {
        let path = Path::new(file);
        ensure!(
            !file.is_empty()
                && path.components().all(|c| {
                    matches!(c, Component::Normal(name) if !name.eq_ignore_ascii_case(".git"))
                }),
            "invalid handoff path {file:?}"
        );
        ensure!(
            scopes.is_empty() || scopes.iter().any(|scope| path.starts_with(scope)),
            "handoff path {file:?} is outside the assigned scope"
        );
        match fs::symlink_metadata(Path::new(dir).join(path)) {
            Ok(metadata) => ensure!(
                metadata.is_file() || metadata.is_symlink(),
                "handoff requires individual files: {file:?}"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    let receipt = format!("Pantheon-Turn: {turn}");
    let current = head(dir)?;
    if current == base {
        let scratch = tempfile::tempdir()?;
        let indexed = |args: &[&str]| {
            output(
                git_command(dir)
                    .env("GIT_INDEX_FILE", scratch.path().join("index"))
                    .args(args),
            )
        };
        indexed(&["read-tree", base])?;
        let mut add = vec!["--literal-pathspecs", "add", "--"];
        add.extend(files.iter().map(String::as_str));
        indexed(&add)?;
        let staged = paths(indexed(&[
            "diff",
            "--cached",
            "--no-renames",
            "--name-only",
            "-z",
            base,
        ])?);
        ensure!(
            !staged.is_empty() && staged.iter().all(|path| files.contains(path)),
            "handoff must name each changed file"
        );
        indexed(&[
            "commit",
            "-m",
            &format!("Complete {task}"),
            "-m",
            summary,
            "-m",
            &receipt,
        ])?;
    } else {
        // A restart may replay the handoff between committing and recording its receipt.
        ensure!(
            git(dir, &["show", "-s", "--format=%P", &current])? == base
                && git(dir, &["show", "-s", "--format=%B", &current])?
                    .trim_end()
                    .ends_with(&receipt),
            "worker HEAD changed before the handoff"
        );
    }
    let mut reset = vec!["--literal-pathspecs", "reset", "--quiet", "HEAD", "--"];
    reset.extend(files.iter().map(String::as_str));
    git(dir, &reset)?;
    head(dir)
}

/// Accept an abbreviated object ID only when Git resolves it to this commit alone.
pub fn matches_commit_id(dir: &str, reported: &str, expected: &str) -> Result<bool> {
    if expected.eq_ignore_ascii_case(reported) {
        return Ok(true);
    }
    if !(4..expected.len()).contains(&reported.len())
        || !reported.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Ok(false);
    }
    let prefix = reported.to_ascii_lowercase();
    if !expected.starts_with(&prefix) {
        return Ok(false);
    }
    Ok(git(dir, &["rev-parse", &format!("--disambiguate={prefix}")])? == expected)
}

pub fn reported_milestone(dir: &str, base: &str) -> Result<String> {
    let head = head(dir)?;
    ensure!(
        head != base && is_ancestor(dir, base, &head),
        "milestone must commit a descendant of its assigned base"
    );
    ensure!(
        !git(dir, &["diff", "--name-only", "-z", base, &head])?.is_empty(),
        "milestone has no changed files"
    );
    Ok(head)
}
pub fn milestone(dir: &str, base: &str) -> Result<String> {
    ensure!(
        status(dir)?.is_empty(),
        "milestone contains uncommitted files"
    );
    reported_milestone(dir, base)
}
pub fn import_commit(target: &str, source: &str, sha: &str, reference: &str) -> Result<()> {
    fetch(target, source, sha)?;
    if let Err(e) = git(
        target,
        &["update-ref", reference, sha, &"0".repeat(sha.len())],
    ) {
        ensure!(
            git(target, &["rev-parse", "--verify", reference])
                .ok()
                .as_deref()
                == Some(sha),
            "{e}"
        );
    }
    Ok(())
}
pub fn merge(dir: &str, sha: &str) -> Result<()> {
    git(
        dir,
        &["-c", "core.hooksPath=/dev/null", "merge", "--no-edit", sha],
    )?;
    Ok(())
}
pub fn diff(dir: &str, base: &str, sha: &str) -> Result<String> {
    const LIMIT: usize = 256 * 1024;
    let mut errors = tempfile::tempfile()?;
    let mut child = git_command(dir)
        .args(["diff", "--no-ext-diff", base, sha, "--"])
        .stdout(Stdio::piped())
        .stderr(errors.try_clone()?)
        .spawn()?;
    let mut buffer = Vec::new();
    let read = child
        .stdout
        .take()
        .context("missing Git output")?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut buffer);
    let truncated = buffer.len() > LIMIT;
    if truncated || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    if truncated {
        buffer.truncate(LIMIT);
        return Ok(format!(
            "{}\n[display truncated]\n",
            String::from_utf8_lossy(&buffer)
        ));
    }
    if !status.success() {
        errors.rewind()?;
        let mut message = Vec::new();
        errors.take(8192).read_to_end(&mut message)?;
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&message).trim_end()
        );
    }
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}
pub fn target_head(repo: &str, reference: &str) -> Result<String> {
    ensure!(
        reference.starts_with("refs/heads/"),
        "choose a local branch"
    );
    git(repo, &["show-ref", "--verify", "--hash", reference])
}
pub fn land(repo: &str, reference: &str, base: &str, candidate: &str) -> Result<()> {
    let current = target_head(repo, reference)?;
    if current == candidate {
        return Ok(());
    }
    ensure!(current == base, "target moved from {base} to {current}");
    ensure!(
        is_ancestor(repo, base, candidate),
        "candidate cannot fast-forward target"
    );
    let worktrees = git(repo, &["worktree", "list", "--porcelain", "-z"])?;
    let mut checkout = "";
    for field in worktrees.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            checkout = path;
        }
        if field.strip_prefix("branch ") != Some(reference) {
            continue;
        }
        fs::metadata(checkout)?;
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
            "BISECT_START",
        ] {
            let path = git(
                checkout,
                &["rev-parse", "--path-format=absolute", "--git-path", name],
            )?;
            ensure!(
                !Path::new(&path).try_exists()?,
                "target checkout has a Git operation in progress"
            );
        }
        ensure!(
            status(checkout)?.is_empty(),
            "target checkout has local changes"
        );
        ensure!(
            git(checkout, &["symbolic-ref", "--quiet", "HEAD"])? == reference,
            "target checkout changed branches"
        );
        ensure!(head(checkout)? == base, "target checkout moved");
        git(
            checkout,
            &["merge", "--ff-only", "--no-overwrite-ignore", candidate],
        )?;
        ensure!(
            target_head(repo, reference)? == candidate,
            "target changed during landing; inspect its current revision"
        );
        return Ok(());
    }
    git(repo, &["update-ref", reference, candidate, base])?;
    Ok(())
}
pub fn files(dir: &str, base: &str) -> Result<String> {
    git(dir, &["ls-tree", "-r", "--name-only", base])
}
pub fn fetch(dir: &str, source: &str, sha: &str) -> Result<()> {
    git(dir, &["fetch", "--quiet", "--no-tags", "--", source, sha])?;
    Ok(())
}
pub fn read_blob(dir: &str, base: &str, path: &str) -> Result<String> {
    git(dir, &["show", &format!("{base}:{path}")])
}
pub fn check_snapshot(dir: &str, expected: &str) -> Result<()> {
    ensure!(
        head(dir)? == expected,
        "checkout commit does not match the candidate"
    );
    ensure!(
        git(
            dir,
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "--untracked-files=no",
                "--ignore-submodules=none"
            ]
        )?
        .is_empty(),
        "candidate has modifications to tracked files"
    );
    Ok(())
}
fn paths(s: String) -> Vec<String> {
    s.split_terminator('\0').map(str::to_owned).collect()
}
pub fn changed_paths(dir: &str, base: &str, sha: &str) -> Result<Vec<String>> {
    Ok(paths(git(
        dir,
        &["diff", "--no-renames", "--name-only", "-z", base, sha, "--"],
    )?))
}
pub fn repair_changes(unresolved: &str, worker: &str, commit: &str) -> Result<Vec<String>> {
    fetch(unresolved, worker, commit)?;
    Ok(paths(git(
        unresolved,
        &[
            "diff",
            "--cached",
            "--no-renames",
            "--name-only",
            "-z",
            commit,
            "--",
        ],
    )?))
}
pub fn conflicted_paths(dir: &str) -> Result<Vec<String>> {
    Ok(paths(git(
        dir,
        &["diff", "--name-only", "--diff-filter=U", "-z"],
    )?))
}
pub fn pin(dir: &str, reference: &str, sha: &str) -> Result<()> {
    git(dir, &["update-ref", reference, sha])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn init(dir: &Path) -> Result<&str> {
        let dir = path_text(dir)?;
        git(dir, &["init", "-b", "main"])?;
        for (key, val) in [
            ("user.name", "Fixture"),
            ("user.email", "test@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            git(dir, &["config", key, val])?;
        }
        Ok(dir)
    }
    #[test]
    fn initial_picker_matches_git_ignores_and_imports_nested_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = path_text(dir.path())?;
        fs::write(dir.path().join(".gitignore"), "*.tmp\n")?;
        fs::create_dir(dir.path().join("nested"))?;
        fs::write(dir.path().join("nested/two words"), "ok")?;
        fs::write(dir.path().join("hidden.tmp"), "ignored")?;
        let choices = initial_choices(root)?;
        assert_eq!(
            choices["files"],
            serde_json::json!([".gitignore", "nested/two words"])
        );
        assert!(!dir.path().join(".git").exists());
        init(dir.path())?;
        let choices = initial_choices(path_text(&dir.path().join("nested"))?)?;
        assert_eq!(choices["root"], path_text(&fs::canonicalize(root)?)?);
        assert_eq!(choices["files"][1], "nested/two words");
        let repo = open(root, &["nested/two words".into()], true)?;
        assert_eq!(git(&repo.root, &["ls-files"])?, "nested/two words");
        assert!(initial_choices(root).is_err());
        Ok(())
    }
    #[test]
    fn recognizes_only_object_ids_for_the_reported_commit() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = init(tmp.path())?;
        git(dir, &["commit", "--allow-empty", "-m", "Base"])?;
        let base = head(dir)?;
        git(dir, &["commit", "--allow-empty", "-m", "Milestone"])?;
        let sha = head(dir)?;
        let short = git(dir, &["rev-parse", "--short", "HEAD"])?;
        assert!(matches_commit_id(dir, &sha, &sha)?);
        assert!(matches_commit_id(dir, &short, &sha)?);
        for invalid in [&base[..7], &sha[..3], "HEAD", "main", "HEAD^", "--all"] {
            assert!(!matches_commit_id(dir, invalid, &sha)?);
        }
        Ok(())
    }

    #[test]
    fn isolate_milestones_and_preserve_dirty_index() -> Result<()> {
        let home = tempfile::tempdir()?;
        let dir = init(home.path())?;
        let repo = open(dir, &[], true)?;
        fs::create_dir(home.path().join("src"))?;
        fs::write(home.path().join("README"), "mine")?;
        git(dir, &["add", "README"])?;
        fs::write(home.path().join("scratch"), "private")?;
        assert_eq!(
            discover(path_text(&home.path().join("src"))?)?.subdir,
            "src"
        );
        let before = status(dir)?;
        let private = tempfile::tempdir()?;
        let checkout = private.path().join("worker");
        let w = path_text(&checkout)?;
        clone_repo(dir, w, &repo.head, "feature")?;
        apply_commit_policy(dir, w)?;
        fs::write(checkout.join("change"), "implementation")?;
        git(w, &["add", "change"])?;
        git(w, &["commit", "-m", "Change"])?;
        let candidate = milestone(w, &repo.head)?;
        import_commit(dir, w, &candidate, "refs/heads/swarm-candidate")?;
        assert!(land(dir, &repo.target, &repo.head, &candidate).is_err());
        assert_eq!(status(dir)?, before);
        git(dir, &["switch", "--detach", &repo.head])?;
        assert!(discover(dir)?.target.is_empty());
        land(dir, &repo.target, &repo.head, &candidate)?;
        assert_eq!(target_head(dir, &repo.target)?, candidate);
        Ok(())
    }
    #[test]
    fn existing_unborn_parent_reports_the_actual_root_without_changing_it() -> Result<()> {
        let home = tempfile::tempdir()?;
        let root = home.path().canonicalize()?;
        let dir = init(&root)?;
        let child = root.join("app");
        fs::create_dir(&child)?;
        fs::write(root.join("private"), "keep")?;
        let err = open(path_text(&child)?, &[], true).unwrap_err().to_string();
        assert!(err.contains(dir) && err.contains(path_text(&child)?));
        assert!(err.contains("belongs to the Git repository"));
        assert!(!child.join(".git").exists());
        assert!(discover(dir)?.head.is_empty());
        assert!(git(dir, &["ls-files"])?.is_empty());
        Ok(())
    }

    #[test]
    fn ready_handoff_preserves_other_edits_and_is_replayable() -> Result<()> {
        let home = tempfile::tempdir()?;
        let dir = init(home.path())?;
        fs::write(home.path().join("remove"), "old")?;
        let repo = open(dir, &["remove".into()], true)?;
        fs::remove_file(home.path().join("remove"))?;
        fs::write(home.path().join("added"), "reviewed")?;
        fs::write(home.path().join("private"), "other")?;
        git(dir, &["add", "private"])?;
        let files = vec!["added".into(), "remove".into()];
        let commit = |wanted: &[String], turn: &str| {
            commit_handoff(dir, &repo.head, "task", turn, wanted, &files, "Checked.")
        };
        assert!(commit(&["private".into()], "first").is_err());
        let sha = commit(&files, "first")?;
        assert_eq!(commit(&files, "first")?, sha);
        assert!(commit(&files, "other").is_err());
        assert_eq!(changed_paths(dir, &repo.head, &sha)?, files);
        assert_eq!(git(dir, &["diff", "--cached", "--name-only"])?, "private");
        Ok(())
    }
    #[test]
    fn explicit_initial_import_and_linked_worktree() -> Result<()> {
        let home = tempfile::tempdir()?;
        let dir = init(home.path())?;
        let sub = home.path().join("app");
        fs::create_dir(&sub)?;
        fs::write(home.path().join(".gitignore"), "secret\n")?;
        fs::write(home.path().join("secret"), "private")?;
        fs::write(sub.join("hello"), "public")?;
        assert!(open(dir, &[], true).is_err());
        assert!(open(dir, &["secret".into()], true).is_err());
        assert!(open(dir, &["../secret".into()], true).is_err());
        let repo = open(
            path_text(&sub)?,
            &["app/hello".into(), ".gitignore".into()],
            true,
        )?;
        assert_eq!(repo.subdir, "app");
        assert!(status(dir)?.is_empty());
        let other = tempfile::tempdir()?;
        let linked = other.path().join("linked");
        git(
            dir,
            &["worktree", "add", "-b", "other", path_text(&linked)?],
        )?;
        let other = discover(path_text(&linked)?)?;
        assert_eq!(other.common, repo.common);
        assert!(land(&other.root, &other.target, "moved", "invalid").is_err());
        Ok(())
    }
    #[test]
    fn display_diff_caps_large_or_non_utf8_content() -> Result<()> {
        let home = tempfile::tempdir()?;
        let dir = init(home.path())?;
        let base = open(dir, &[], true)?.head;
        let data = (0..100_000)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        fs::write(home.path().join("large.txt"), data)?;
        git(dir, &["add", "large.txt"])?;
        git(dir, &["commit", "-m", "Add large fixture"])?;
        let display = diff(dir, &base, &head(dir)?)?;
        assert!(display.ends_with("[display truncated]\n"));
        assert!(display.len() < 263_000);
        fs::write(home.path().join("raw.txt"), [0xff, b'\n'])?;
        git(dir, &["add", "raw.txt"])?;
        git(dir, &["commit", "-m", "Add non-UTF-8 content"])?;
        assert!(diff(dir, "HEAD^", "HEAD")?.contains('�'));
        assert!(diff(dir, "missing", "HEAD").is_err());
        Ok(())
    }

    #[test]
    fn conflict_repair_only_touches_unresolved_paths() -> Result<()> {
        let home = tempfile::tempdir()?;
        let dir = init(home.path())?;
        fs::write(home.path().join("overlap"), "base")?;
        git(dir, &["add", "."])?;
        git(dir, &["commit", "-m", "Base"])?;
        git(dir, &["switch", "-c", "right"])?;
        fs::write(home.path().join("overlap"), "right")?;
        fs::write(home.path().join("clean"), "right only")?;
        git(dir, &["add", "."])?;
        git(dir, &["commit", "-m", "Right"])?;
        let right = head(dir)?;
        git(dir, &["switch", "main"])?;
        fs::write(home.path().join("overlap"), "left")?;
        git(dir, &["commit", "-am", "Left"])?;
        assert!(merge(dir, &right).is_err());
        assert_eq!(conflicted_paths(dir)?, ["overlap"]);
        let other = tempfile::tempdir()?;
        let repair = other.path().join("repair");
        let w = path_text(&repair)?;
        clone_repo(dir, w, &head(dir)?, "repair")?;
        apply_commit_policy(dir, w)?;
        assert!(merge(w, &right).is_err());
        fs::write(repair.join("overlap"), "both")?;
        git(w, &["commit", "-am", "Resolve"])?;
        assert_eq!(repair_changes(dir, w, &head(w)?)?, ["overlap"]);
        fs::write(repair.join("clean"), "unexpected")?;
        git(w, &["commit", "-am", "Extra"])?;
        assert_eq!(repair_changes(dir, w, &head(w)?)?, ["clean", "overlap"]);
        Ok(())
    }
}
