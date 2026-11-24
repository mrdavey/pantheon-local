//! A bounded projection of assistant text for one live coordinator turn.
use serde::de::IgnoredAny;
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

const LIMIT: usize = 128 * 1024;
const FILE: &str = "chat.partial";

pub struct Writer {
    file: File,
    length: usize,
    message: String,
}

impl Writer {
    pub fn create(dir: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(dir.join(FILE))?;
        Ok(Self {
            file,
            length: 0,
            message: String::new(),
        })
    }

    pub fn event(&mut self, event: &Value) -> io::Result<()> {
        let (text, message) = match event["type"].as_str() {
            Some("agent_message_chunk") => (
                event["text"].as_str(),
                event["raw"]["update"]["messageId"]
                    .as_str()
                    .or(event["messageId"].as_str()),
            ),
            Some("stream_event") => {
                let raw = &event["raw"]["event"];
                if raw["type"] == "message_start" {
                    self.reset(raw["message"]["id"].as_str().unwrap_or_default())?;
                }
                if raw["type"] != "content_block_delta" || raw["delta"]["type"] != "text_delta" {
                    return Ok(());
                }
                (raw["delta"]["text"].as_str(), None)
            }
            _ => return Ok(()),
        };
        if let Some(id) = message
            && id != self.message
        {
            self.reset(id)?;
        }
        if let Some(text) = text {
            let n = text.floor_char_boundary(text.len().min(LIMIT - self.length));
            self.file.write_all(&text.as_bytes()[..n])?;
            self.length += n;
        }
        Ok(())
    }

    fn reset(&mut self, id: &str) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.rewind()?;
        self.length = 0;
        self.message = id.to_owned();
        Ok(())
    }
}

pub fn preview(dir: &Path) -> io::Result<String> {
    let mut raw = Vec::new();
    match File::open(dir.join(FILE)) {
        Ok(file) => {
            file.take(LIMIT as u64).read_to_end(&mut raw)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e),
    }
    let mut text = reply_prefix(&String::from_utf8_lossy(&raw)).unwrap_or_default();
    text.truncate(text.floor_char_boundary(text.len().min(12_000)));
    Ok(text)
}

// Parse complete top-level keys, then decode only complete characters of the reply.
// A chunk may end inside a JSON escape or a UTF-16 surrogate pair.
fn reply_prefix(raw: &str) -> Option<String> {
    let raw = raw.trim_start();
    let mut input = raw
        .strip_prefix("```json\n")
        .unwrap_or(raw)
        .trim_start()
        .strip_prefix('{')?;
    loop {
        let mut key = serde_json::Deserializer::from_str(input).into_iter::<String>();
        let name = key.next()?.ok()?;
        input = input[key.byte_offset()..]
            .trim_start()
            .strip_prefix(':')?
            .trim_start();
        if name == "reply" {
            let content = input.strip_prefix('"')?;
            let mut escaped = false;
            for (i, byte) in content.bytes().enumerate() {
                if byte == b'"' && !escaped {
                    return serde_json::from_str(&input[..i + 2]).ok();
                }
                escaped = byte == b'\\' && !escaped;
            }
            let mut partial = input.to_owned();
            // One JSON character can span two six-byte escapes.
            for _ in 0..=12 {
                partial.push('"');
                if let Ok(text) = serde_json::from_str::<String>(&partial) {
                    return Some(text);
                }
                partial.pop();
                partial.pop()?;
            }
            return None;
        }
        let mut value = serde_json::Deserializer::from_str(input).into_iter::<IgnoredAny>();
        value.next()?.ok()?;
        input = input[value.byte_offset()..]
            .trim_start()
            .strip_prefix(',')?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_native_message_boundaries_and_ignores_non_reply_events() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut writer = Writer::create(dir.path())?;
        let chunk = |kind, id, text| serde_json::json!({"type":kind, "raw":{"update":{"messageId":id}}, "text":text});
        writer.event(&chunk("agent_message_chunk", "first", r#"{"reply":"First"#))?;
        writer.event(&chunk("agent_thought_chunk", "first", "private"))?;
        assert_eq!(preview(dir.path())?, "First");
        writer.event(&chunk(
            "agent_message_chunk",
            "second",
            r#"{"reply":"Second"#,
        ))?;
        assert_eq!(preview(dir.path())?, "Second");
        let claude = |event| serde_json::json!({"type":"stream_event","raw":{"event":event}});
        writer.event(&claude(
            serde_json::json!({"type":"message_start","message":{"id":"message"}}),
        ))?;
        writer.event(&claude(serde_json::json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"private"}})))?;
        assert!(preview(dir.path())?.is_empty());
        writer.event(&claude(serde_json::json!({"type":"content_block_delta","delta":{"type":"text_delta","text":r#"{"reply":"Claude"#}})))?;
        assert_eq!(preview(dir.path())?, "Claude");
        Ok(())
    }

    #[test]
    fn streams_only_the_top_level_reply_and_handles_split_unicode() {
        let raw =
            r#"{"tasks":[{"reply":"private"}], "reply":"OK\n\ud83d\ude42 'test'","checks":[]}"#;
        for (i, _) in raw.char_indices() {
            let part = reply_prefix(&raw[..i]).unwrap_or_default();
            assert!("OK\n🙂 'test'".starts_with(&part), "at {i}: {part:?}");
            if raw[..i].contains("OK\n") {
                assert!(part.starts_with("OK"), "lost text at {i}");
            }
        }
        assert_eq!(reply_prefix(raw).as_deref(), Some("OK\n🙂 'test'"));
        assert_eq!(
            reply_prefix(r#"{"reply":"Hello "#).as_deref(),
            Some("Hello ")
        );
        assert_eq!(reply_prefix("private thoughts"), None);
    }
}
