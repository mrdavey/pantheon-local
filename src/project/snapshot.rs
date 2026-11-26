//! Read an implementation directly from its integration commit, including for subprojects.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub fn snapshot(repo: &Path, commit: &str, subdir: &str, path: &str) -> Result<Value> {
    ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid implementation commit"
    );
    let mut segments = Vec::new();
    for part in [if subdir == "." { "" } else { subdir }, path] {
        if part.is_empty() {
            continue;
        }
        for name in part.split('/') {
            ensure!(
                !matches!(name, "" | "." | "..") && !name.contains('\0'),
                "invalid project path"
            );
            segments.push(name);
        }
    }
    let dir = path_text(repo)?;
    let object = format!("{commit}:{}", segments.join("/"));
    match git(dir, &["cat-file", "-t", &object])?.as_str() {
        "tree" => {
            let list = git(dir, &["ls-tree", "-z", &object])?;
            let mut entries = Vec::new();
            for row in list.split_terminator('\0') {
                let (meta, name) = row.split_once('\t').context("invalid tree entry")?;
                let kind = meta
                    .split_whitespace()
                    .nth(1)
                    .context("missing object type")?;
                entries.push(json!({"name":name,"kind":kind}));
            }
            Ok(json!({"kind":"tree","entries":entries}))
        }
        "blob" => {
            let size: usize = git(dir, &["cat-file", "-s", &object])?.parse()?;
            if size > 512 * 1024 {
                return Ok(
                    json!({"kind":"unavailable","reason":"This file exceeds the 512 KiB preview limit."}),
                );
            }
            let out = git_command(dir)
                .args(["cat-file", "blob", &object])
                .output()?;
            ensure!(out.status.success(), "could not read committed file");
            let mime = match out.stdout.as_slice() {
                [0x89, b'P', b'N', b'G', 13, 10, 26, 10, ..] => Some("image/png"),
                [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
                [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
                bytes if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => {
                    Some("image/webp")
                }
                _ => None,
            };
            if let Some(mime) = mime {
                return Ok(json!({"kind":"image","mime":mime,"bytes":STANDARD.encode(out.stdout)}));
            }
            if !out.stdout.contains(&0)
                && let Ok(text) = std::str::from_utf8(&out.stdout)
            {
                return Ok(json!({"kind":"text","text":text}));
            }
            Ok(
                json!({"kind":"unavailable","reason":"Binary file. Open the project folder to view it."}),
            )
        }
        _ => Ok(json!({"kind":"unavailable","reason":"This entry points to another repository."})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_stays_inside_the_committed_project_tree() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let dir = path_text(root)?;
        git(dir, &["init", "-q"])?;
        fs::create_dir(root.join("app"))?;
        fs::write(root.join("app/name #.md"), "# Before\n")?;
        fs::write(root.join("app/image"), [0, 255])?;
        fs::write(root.join("outside"), "private")?;
        std::os::unix::fs::symlink("../outside", root.join("app/link"))?;
        git(dir, &["add", "."])?;
        git(
            dir,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "snapshot",
            ],
        )?;
        let commit = git(dir, &["rev-parse", "HEAD"])?;
        fs::write(root.join("app/name #.md"), "# After")?;
        let view = |path| snapshot(root, &commit, "app", path);
        assert_eq!(view("")?["entries"].as_array().unwrap().len(), 3);
        assert_eq!(view("name #.md")?["text"], "# Before\n");
        assert_eq!(view("image")?["kind"], "unavailable");
        assert_eq!(view("link")?["text"], "../outside");
        assert!(view("../outside").is_err());
        Ok(())
    }
}
