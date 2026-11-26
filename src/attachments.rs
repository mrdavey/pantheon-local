//! Brief files are private run inputs. Store bytes once, outside all Git checkouts.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};

const TOTAL: usize = 16 * 1024 * 1024;
pub const REQUEST_LIMIT: usize = 23 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    name: String,
    data: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

pub fn unpack(input: &mut Value) -> Result<Vec<(String, Vec<u8>)>> {
    let Some(value) = input.as_object_mut().and_then(|o| o.remove("attachments")) else {
        return Ok(Vec::new());
    };
    let uploads: Vec<Upload> = serde_json::from_value(value)?;
    ensure!(uploads.len() <= 16, "Attach up to 16 files.");
    let mut total = 0;
    let mut files = Vec::new();
    for Upload { name, data } in uploads {
        ensure!(valid_name(&name), "Invalid attachment name.");
        let bytes = STANDARD.decode(data).context("Invalid attachment data.")?;
        total += bytes.len();
        ensure!(
            total <= TOTAL,
            "Attachments must fit within 16 MiB in total."
        );
        files.push((name, bytes));
    }
    Ok(files)
}

pub fn read(root: &Path, files: &Value, index: usize) -> Result<Value> {
    let file = files.get(index).context("Attachment not found.")?;
    let name = file["name"]
        .as_str()
        .filter(|name| valid_name(name))
        .context("Invalid attachment name.")?;
    let relative = Path::new("attachments").join(index.to_string()).join(name);
    ensure!(
        relative.to_str() == file["path"].as_str(),
        "Invalid attachment path."
    );
    let path = root.canonicalize()?.join(relative);
    ensure!(
        path.canonicalize().ok().as_ref() == Some(&path) && fs::symlink_metadata(&path)?.is_file(),
        "Attachment is unavailable."
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(TOTAL as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= TOTAL, "Attachment exceeds 16 MiB.");
    Ok(json!({"data": STANDARD.encode(bytes)}))
}

pub fn save(root: &Path, files: Vec<(String, Vec<u8>)>) -> Result<Value> {
    if files.is_empty() {
        return Ok(json!([]));
    }
    let staging = tempfile::tempdir_in(root)?;
    let mut entries = Vec::new();
    for (index, (name, bytes)) in files.into_iter().enumerate() {
        let relative = Path::new("attachments").join(index.to_string()).join(&name);
        let directory = staging.path().join(index.to_string());
        fs::create_dir(&directory)?;
        fs::write(directory.join(&name), &bytes)?;
        entries.push(json!({"name":name, "size":bytes.len(), "path":relative}));
    }
    fs::rename(staging.path(), root.join("attachments"))?;
    Ok(Value::Array(entries))
}

/// Text excerpts also reach adapters that cannot read files in a planning turn.
pub fn context(root: &Path, files: &Value) -> Result<Value> {
    let mut result = Vec::new();
    for file in files.as_array().into_iter().flatten() {
        let path = root.join(file["path"].as_str().context("Missing attachment path")?);
        let mut prefix = Vec::new();
        fs::File::open(&path)?.take(4096).read_to_end(&mut prefix)?;
        let mut entry = json!({"name":file["name"],"size":file["size"],"path":path});
        if !prefix.contains(&0) {
            let sample = match std::str::from_utf8(&prefix) {
                Ok(s) => Some(s),
                Err(e) if e.error_len().is_none() => {
                    std::str::from_utf8(&prefix[..e.valid_up_to()]).ok()
                }
                _ => None,
            };
            if let Some(text) = sample {
                entry["excerpt"] = json!(text);
            }
        }
        result.push(entry);
    }
    Ok(result.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_names_and_binary_files_remain_separate_from_project_and_run_metadata() -> Result<()>
    {
        let mut input = json!({"prompt":"build","attachments":[{"name":"same","data":"aGk="},{"name":"same","data":"AP8="}]});
        let root = tempfile::tempdir()?;
        let metadata = save(root.path(), unpack(&mut input)?)?;
        assert!(input.get("attachments").is_none());
        assert!(!metadata.to_string().contains("aGk="));
        let ctx = context(root.path(), &metadata)?;
        assert_eq!(ctx[0]["excerpt"], "hi");
        assert!(ctx[1].get("excerpt").is_none());
        assert_eq!(fs::read(ctx[1]["path"].as_str().unwrap())?, vec![0, 255]);
        assert_eq!(read(root.path(), &metadata, 0)?["data"], "aGk=");
        assert_eq!(read(root.path(), &metadata, 1)?["data"], "AP8=");
        assert!(read(root.path(), &metadata, 2).is_err());
        let path = root.path().join("attachments/1/same");
        fs::remove_file(&path)?;
        std::os::unix::fs::symlink(root.path().join("attachments/0/same"), path)?;
        assert!(read(root.path(), &metadata, 1).is_err());
        assert!(unpack(&mut json!({"attachments":[{"name":"../x","data":""}]})).is_err());
        assert!(unpack(&mut json!({"attachments":[{"name":"bad","data":"?"}]})).is_err());
        Ok(())
    }
}
