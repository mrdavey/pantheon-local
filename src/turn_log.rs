//! Reverse pages of append-only turn output. Cursors are exclusive byte offsets.
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::{fs::FileExt, fs::OpenOptionsExt},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

const PAGE: usize = 25;
const BLOCK: usize = 16 * 1024;
const PAGE_BYTES: usize = 512 * 1024;
// An 8 MiB native event can include the same text in the raw and normalized fields.
const MAX_ENTRY: u64 = 24 * 1024 * 1024;

struct Lines<'a> {
    file: &'a File,
    start: u64,
    remaining: usize,
    block: Vec<u8>,
}
impl<'a> Lines<'a> {
    fn new(file: &'a File, end: u64) -> Self {
        Self {
            file,
            start: end,
            remaining: 0,
            block: vec![0; BLOCK],
        }
    }
    fn offset(&self) -> u64 {
        self.start + self.remaining as u64
    }
    fn fill(&mut self) -> io::Result<bool> {
        if self.remaining == 0 && self.start > 0 {
            self.remaining = self.start.min(BLOCK as u64) as usize;
            self.start -= self.remaining as u64;
            self.file
                .read_exact_at(&mut self.block[..self.remaining], self.start)?;
        }
        Ok(self.remaining > 0)
    }
    fn previous(&mut self) -> io::Result<Option<(u64, u64)>> {
        if !self.fill()? {
            return Ok(None);
        }
        if self.block[self.remaining - 1] == b'\n' {
            self.remaining -= 1;
        }
        let end = self.offset();
        while self.fill()? {
            if let Some(i) = self.block[..self.remaining]
                .iter()
                .rposition(|b| *b == b'\n')
            {
                self.remaining = i + 1;
                break;
            }
            self.remaining = 0;
        }
        Ok(Some((self.offset(), end)))
    }
}

// External checks and release tools write directly to events.log. Record when
// appended ranges were observed without changing their stdout/stderr contract.
pub fn observe_output<T>(dir: &Path, run: impl FnOnce() -> T) -> T {
    thread::scope(|scope| {
        let (stop, wait) = mpsc::channel::<()>();
        scope.spawn(move || {
            let path = dir.join("events.log");
            let mut index = None;
            let mut last = 0;
            loop {
                let done = matches!(
                    wait.recv_timeout(Duration::from_millis(250)),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                if let Ok(meta) = path.metadata()
                    && meta.len() > last
                {
                    if index.is_none() {
                        index = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .mode(0o600)
                            .open(dir.join("output.times"))
                            .ok();
                    }
                    if let Some(f) = &mut index {
                        let mut record = [0; 16];
                        record[..8].copy_from_slice(&meta.len().to_le_bytes());
                        record[8..]
                            .copy_from_slice(&chrono::Utc::now().timestamp_millis().to_le_bytes());
                        if f.write_all(&record).is_err() {
                            return;
                        }
                    }
                    last = meta.len();
                }
                if done {
                    break;
                }
            }
        });
        let result = run();
        drop(stop);
        result
    })
}
fn observed(index: Option<&File>, offset: u64) -> Option<i64> {
    let f = index?;
    let mut low = 0;
    let mut high = f.metadata().ok()?.len() / 16;
    let mut stamp = None;
    while low < high {
        let mid = low + (high - low) / 2;
        let mut data = [0; 16];
        f.read_exact_at(&mut data, mid * 16).ok()?;
        if u64::from_le_bytes(data[..8].try_into().ok()?) > offset {
            high = mid;
            stamp = Some(i64::from_le_bytes(data[8..].try_into().ok()?));
        } else {
            low = mid + 1;
        }
    }
    stamp
}
fn hidden(e: &Value) -> bool {
    let raw = &e["raw"];
    matches!(
        e["type"].as_str(),
        Some("agent_thought_chunk" | "usage" | "system" | "stream_event")
    ) || raw["item"]["type"] == "reasoning"
        || raw["message"]["content"].as_array().is_some_and(|blocks| {
            blocks
                .iter()
                .all(|b| matches!(b["type"].as_str(), Some("thinking" | "redacted_thinking")))
        })
}
fn timestamp(e: &Value) -> Option<i64> {
    e["logged_at"].as_i64().or_else(|| {
        [&e["timestamp"], &e["raw"]["timestamp"]]
            .into_iter()
            .filter_map(Value::as_str)
            .find_map(|v| {
                chrono::DateTime::parse_from_rfc3339(v)
                    .ok()
                    .map(|t| t.timestamp_millis())
            })
    })
}
struct Message {
    row: Value,
    chunks: Vec<String>,
    chunk_bytes: usize,
    key: Option<(String, String)>,
}
impl Message {
    fn finish(mut self) -> Value {
        if self.key.is_some() {
            self.chunks.reverse();
            self.row["event"]["text"] = self.chunks.concat().into();
        }
        self.row
    }
}
pub fn page(dir: &Path, before: Option<u64>, known: Option<u64>, running: bool) -> Result<Value> {
    let file = match File::open(dir.join("events.log")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(json!({"end":0,"older":null,"entries":[]}));
        }
        Err(e) => return Err(e.into()),
    };
    let size = file.metadata()?.len();
    let end = before.unwrap_or(size).min(size);
    if known == Some(end) {
        return Ok(json!({"end":end,"unchanged":true}));
    }
    let mut lines = Lines::new(&file, end);
    let times = File::open(dir.join("output.times")).ok();
    let mut rows: Vec<Message> = Vec::new();
    let mut older = None;
    let mut page_bytes = 0;
    while let Some((start, stop)) = lines.previous()? {
        if stop == start || ((running || end < size) && stop == end) {
            continue;
        } // unfinished trailing write
        let (event, line) = if stop - start > MAX_ENTRY {
            (
                Value::Null,
                Some(format!(
                    "Log entry exceeds the display limit ({} KiB). Full output is in events.log.",
                    (stop - start).div_ceil(1024)
                )),
            )
        } else {
            let mut bytes = vec![0; (stop - start) as usize];
            file.read_exact_at(&mut bytes, start)?;
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(event) if event["type"].is_string() => (event, None),
                _ => (
                    Value::Null,
                    Some(String::from_utf8_lossy(&bytes).into_owned()),
                ),
            }
        };
        if hidden(&event) {
            continue;
        }
        let chunk = (event["type"] == "agent_message_chunk").then(|| {
            (
                event["session"].as_str().unwrap_or("").to_owned(),
                event["messageId"]
                    .as_str()
                    .or(event["raw"]["update"]["messageId"].as_str())
                    .unwrap_or("")
                    .into(),
            )
        });
        let text = event["text"].as_str().unwrap_or("");
        if chunk.is_some()
            && let Some(previous) = rows.last_mut()
            && previous.key == chunk
            && previous.chunk_bytes + text.len() <= PAGE_BYTES
        {
            if !text.is_empty() {
                previous.chunks.push(text.into());
            }
            page_bytes += text.len();
            previous.chunk_bytes += text.len();
            previous.row["id"] = start.into();
            continue;
        }
        if rows.len() == PAGE || (!rows.is_empty() && page_bytes >= PAGE_BYTES) {
            older = Some(stop + 1);
            break;
        }
        page_bytes += (stop - start).min(MAX_ENTRY) as usize;
        let at = timestamp(&event).or_else(|| observed(times.as_ref(), stop - 1));
        rows.push(Message {
            row: json!({"id":start,"at":at,"event":event,"line":line}),
            chunks: if chunk.is_some() {
                vec![text.into()]
            } else {
                vec![]
            },
            chunk_bytes: text.len(),
            key: chunk,
        });
    }
    Ok(
        json!({"end":end,"older":older,"entries":rows.into_iter().map(Message::finish).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn pages_keep_their_place_after_appends() -> Result<()> {
        let dir = tempdir()?;
        let mut log = File::create(dir.path().join("events.log"))?;
        // Each line crosses more than one read block; the first entries are
        // outside the legacy evidence tail.
        for i in 0..56 {
            writeln!(log, "{i} {}", "x".repeat(BLOCK + 3))?;
        }
        let first = page(dir.path(), None, None, true)?;
        let cursor = first["older"].as_u64();
        assert!(
            first["entries"][0]["line"]
                .as_str()
                .unwrap()
                .starts_with("55 ")
        );
        assert_eq!(first["entries"].as_array().unwrap().len(), 25);
        assert_eq!(
            page(dir.path(), None, first["end"].as_u64(), true)?["unchanged"],
            true
        );
        writeln!(log, "new arrival")?;
        let second = page(dir.path(), cursor, None, true)?;
        let last = page(dir.path(), second["older"].as_u64(), None, true)?;
        assert!(
            second["entries"][0]["line"]
                .as_str()
                .unwrap()
                .starts_with("30 ")
        );
        assert!(
            last["entries"][5]["line"]
                .as_str()
                .unwrap()
                .starts_with("0 ")
        );
        assert!(last["older"].is_null());
        assert_eq!(
            page(dir.path(), first["end"].as_u64(), None, true)?["entries"],
            first["entries"]
        );
        Ok(())
    }

    #[test]
    fn joins_public_message_fragments_and_ignores_unfinished_writes() -> Result<()> {
        let dir = tempdir()?;
        let mut log = File::create(dir.path().join("events.log"))?;
        writeln!(log, "historical output")?;
        writeln!(
            log,
            "{}",
            json!({"type":"agent_message_chunk","messageId":"A","text":"Hello "})
        )?;
        writeln!(
            log,
            "{}",
            json!({"type":"agent_thought_chunk","text":"hidden"})
        )?;
        writeln!(
            log,
            "{}",
            json!({"type":"agent_message_chunk","messageId":"A","text":"world","timestamp":"2026-01-01T00:00:00Z"})
        )?;
        write!(log, "par")?;
        let live = page(dir.path(), None, None, true)?;
        assert_eq!(live["entries"].as_array().unwrap().len(), 2);
        assert_eq!(live["entries"][0]["event"]["text"], "Hello world");
        assert_eq!(live["entries"][0]["at"], 1767225600000_i64);
        assert!(live["entries"][1]["at"].is_null());
        write!(log, "tial")?;
        assert_eq!(
            page(dir.path(), live["end"].as_u64(), None, false)?["entries"],
            live["entries"]
        );
        assert_eq!(
            page(dir.path(), None, None, false)?["entries"][0]["line"],
            "partial"
        );
        Ok(())
    }

    #[test]
    fn records_observation_times_for_external_output() -> Result<()> {
        let dir = tempdir()?;
        assert_eq!(page(dir.path(), None, None, true)?["entries"], json!([]));
        let start = chrono::Utc::now().timestamp_millis();
        observe_output(dir.path(), || {
            fs::write(
                dir.path().join("events.log"),
                b"[check] cargo test\ncheck passed\n",
            )
        })?;
        let p = page(dir.path(), None, None, false)?;
        assert_eq!(p["entries"][0]["line"], "check passed");
        for row in p["entries"].as_array().unwrap() {
            assert!(row["at"].as_i64().is_some_and(|at| at >= start));
        }
        Ok(())
    }
}
