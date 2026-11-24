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
    if !dir.is_empty() {
        cmd.current_dir(dir);
    }
    for (key, _) in env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("LC_ALL", "C");
    cmd
}

fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().context("running Git")?;
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
    let path = fs::canonicalize(path)?;
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
            git(path_text(&canonical)?, &["init", "--quiet"])?;
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
    ensure!(
        !files.is_empty()
            || git(
                &repo.root,
                &["ls-files", "--others", "--exclude-standard", "-z"]
            )?
            .is_empty(),
        "choose initial files explicitly (or commit in Git)"
    );
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
    fs::create_dir_all(Path::new(dir).parent().context("missing clone parent")?)?;
    git(
        "",
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
