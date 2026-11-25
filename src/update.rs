//! Check GitHub Releases for newer Pantheon Local versions with local caching.
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{
    env,
    fs::{self, File},
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::Path,
    time::Duration,
};

const FEED: &str = "https://api.github.com/repos/mrdavey/pantheon-local/releases/latest";
const RELEASES_PREFIX: &str = "https://github.com/mrdavey/pantheon-local/releases/";
const FALLBACK_URL: &str = "https://github.com/mrdavey/pantheon-local/releases/latest";
const CACHE_FILE: &str = "update-check.json";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn endpoint() -> Option<String> {
    if env::var("SWARM_NO_UPDATE_CHECK")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
    {
        return None;
    }
    match env::var("SWARM_UPDATE_URL") {
        Ok(val) => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                Some(FEED.to_owned())
            } else if matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "off" | "0" | "false" | "none"
            ) {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Err(_) => Some(FEED.to_owned()),
    }
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let s = raw.trim().trim_start_matches(['v', 'V']);
    let s = s.split('+').next()?;
    if s.contains('-') {
        return None;
    }
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn clean_version(raw: &str) -> Option<String> {
    let (maj, min, pat) = parse_semver(raw)?;
    Some(format!("{maj}.{min}.{pat}"))
}

pub fn is_newer(current: &str, candidate: &str) -> bool {
    matches!(
        (parse_semver(current), parse_semver(candidate)),
        (Some(cur), Some(cand)) if cand > cur
    )
}

fn safe_release_url(raw: &str) -> String {
    if let Ok(url) = Url::parse(raw.trim())
        && url.as_str().starts_with(RELEASES_PREFIX)
        && url.username().is_empty()
        && url.password().is_none()
    {
        return url.into();
    }
    FALLBACK_URL.to_owned()
}

fn bounded_json(mut reader: impl Read) -> Result<Value> {
    const LIMIT: u64 = 256 << 10;
    let mut buf = Vec::new();
    (&mut reader).take(LIMIT + 1).read_to_end(&mut buf)?;
    ensure!(
        buf.len() as u64 <= LIMIT,
        "release metadata exceeds size limit"
    );
    Ok(serde_json::from_slice(&buf)?)
}

fn notice_from(current: &str, data: &Value) -> Option<Value> {
    let latest = clean_version(data["latest"].as_str()?)?;
    if !is_newer(current, &latest) {
        return None;
    }
    let url = safe_release_url(data["url"].as_str().unwrap_or_default());
    Some(json!({
        "current": current,
        "latest": latest,
        "url": url,
    }))
}

fn read_cache(path: &Path) -> Option<Value> {
    File::open(path)
        .ok()
        .and_then(|f| bounded_json(f).ok())
        .filter(Value::is_object)
}

fn cache_fresh(data: &Value, now: DateTime<Utc>) -> bool {
    let Some(checked) = data["checked"]
        .as_str()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
    else {
        return false;
    };
    let age = now.signed_duration_since(checked);
    let ttl = if data["latest"].as_str().is_some_and(|s| !s.is_empty()) {
        chrono::Duration::hours(12)
    } else {
        chrono::Duration::hours(1)
    };
    (chrono::Duration::zero()..ttl).contains(&age)
}

fn fetch_latest(url: &str) -> Result<(String, String)> {
    let parsed = Url::parse(url)?;
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_millis(1500));
    if parsed
        .host_str()
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost")
    {
        builder = builder.no_proxy();
    }
    let response = builder
        .build()?
        .get(parsed)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            concat!("pantheon-local/", env!("CARGO_PKG_VERSION")),
        )
        .send()?;
    ensure!(
        response.status().as_u16() == 200,
        "release check HTTP {}",
        response.status()
    );
    let body = bounded_json(response)?;
    ensure!(
        body["draft"] != true && body["prerelease"] != true,
        "latest release is a draft or prerelease"
    );
    let latest = body["tag_name"]
        .as_str()
        .and_then(clean_version)
        .ok_or_else(|| anyhow::anyhow!("missing release version tag"))?;
    let html_url = safe_release_url(body["html_url"].as_str().unwrap_or_default());
    Ok((latest, html_url))
}

/// Read only already-cached update state so startup paths never wait on the network.
pub fn cached(state: &Path) -> Option<Value> {
    endpoint()?;
    let data = read_cache(&state.join(CACHE_FILE))?;
    notice_from(CURRENT, &data)
}

/// Return an update notice if a newer release exists, refreshing the local cache when stale.
pub fn check(state: &Path) -> Option<Value> {
    let url = endpoint()?;
    check_with(state, &url, CURRENT, Utc::now())
}

fn check_with(state: &Path, url: &str, current: &str, now: DateTime<Utc>) -> Option<Value> {
    let cache_path = state.join(CACHE_FILE);
    let existing = read_cache(&cache_path);
    if let Some(ref data) = existing
        && cache_fresh(data, now)
    {
        return notice_from(current, data);
    }
    let next = match fetch_latest(url) {
        Ok((latest, release_url)) => {
            json!({"checked": now.to_rfc3339(), "latest": latest, "url": release_url})
        }
        Err(_) => {
            let prev_latest = existing
                .as_ref()
                .and_then(|d| d["latest"].as_str())
                .unwrap_or_default();
            let prev_url = existing
                .as_ref()
                .and_then(|d| d["url"].as_str())
                .unwrap_or_default();
            json!({"checked": now.to_rfc3339(), "latest": prev_latest, "url": prev_url})
        }
    };
    if fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(state)
        .is_ok()
    {
        let _ = crate::supervisor::write_json(&cache_path, &next);
    }
    notice_from(current, &next)
}

pub fn cli_message(update: &Value) -> Option<String> {
    let latest = update["latest"].as_str()?;
    let current = update["current"].as_str().unwrap_or(CURRENT);
    let url = update["url"].as_str().unwrap_or(FALLBACK_URL);
    Some(format!(
        "Update available: Pantheon Local v{latest} (current v{current}) — {url}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn compares_stable_semver_tags_and_filters_prereleases_or_untrusted_urls() {
        assert!(is_newer("0.3.0", "v0.3.1"));
        assert!(is_newer("0.3.9", "v0.4.0"));
        assert!(is_newer("0.4.0", "1.0.0"));
        assert!(!is_newer("0.4.0", "v0.4.0"));
        assert!(!is_newer("0.4.0", "v0.3.9"));
        assert!(!is_newer("0.4.0", "v0.5.0-rc.1"));
        assert_eq!(
            safe_release_url("https://github.com/mrdavey/pantheon-local/releases/tag/v0.4.0"),
            "https://github.com/mrdavey/pantheon-local/releases/tag/v0.4.0"
        );
        assert_eq!(
            safe_release_url("https://evil.example/releases/tag/v0.4.0"),
            FALLBACK_URL
        );
    }

    #[test]
    fn caches_release_check_and_serves_update_notice_without_repolling() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let server = thread::spawn(move || -> Result<()> {
            let (mut sock, _) = listener.accept()?;
            let mut reader = BufReader::new(&sock);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if line.trim().is_empty() {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("user-agent: pantheon-local/")
            );
            let body = json!({
                "tag_name": "v9.2.0",
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/mrdavey/pantheon-local/releases/tag/v9.2.0"
            })
            .to_string();
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
            Ok(())
        });

        let t0: DateTime<Utc> = "2026-11-25T12:00:00Z".parse()?;
        let notice = check_with(dir.path(), &endpoint, "0.4.0", t0).expect("update notice");
        server.join().unwrap()?;
        assert_eq!(notice["current"], "0.4.0");
        assert_eq!(notice["latest"], "9.2.0");
        assert_eq!(
            notice["url"],
            "https://github.com/mrdavey/pantheon-local/releases/tag/v9.2.0"
        );
        assert!(
            cli_message(&notice)
                .unwrap()
                .contains("https://github.com/mrdavey/pantheon-local/releases/tag/v9.2.0")
        );

        // Within the 12-hour TTL, check_with uses the cached result even when the server is closed.
        let cached_notice = check_with(
            dir.path(),
            "http://127.0.0.1:1",
            "0.4.0",
            t0 + chrono::Duration::hours(2),
        )
        .expect("cached notice");
        assert_eq!(cached_notice, notice);

        // If already on the latest version, no notice is returned.
        assert!(check_with(dir.path(), "http://127.0.0.1:1", "9.2.0", t0).is_none());
        Ok(())
    }
}
