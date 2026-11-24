//! Developer-approved checks and immutable evaluator programs.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    env,
    fs::{self, File},
    io::{Read, Seek, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use wait_timeout::ChildExt;

const MAX_DRIVER: u64 = 4 << 20;
const MAX_RECEIPT: u64 = 256 << 10;
const TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Default, Deserialize)]
#[serde(default)]
struct Recipe {
    cases: Option<Vec<Case>>,
    checks: Option<Vec<Vec<String>>>,
    preview: Option<Vec<String>>,
    rule: Option<Rule>,
    rubric: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Case {
    id: String,
    revision: u32,
    driver: String,
    program: String,
    interpreter: String,
    digest: String,
    input: Value,
    expected: Value,
    samples: u32,
    seed: i64,
    required: bool,
    holdback: bool,
}
#[derive(Deserialize)]
struct Rule {
    metrics: Vec<Metric>,
    objective: String,
    #[serde(default)]
    minimum: f64,
    #[serde(default)]
    min_lead: f64,
    #[serde(default)]
    min_samples: u32,
    #[serde(default)]
    tie_break: String,
    #[serde(default)]
    limits: BTreeMap<String, f64>,
}
#[derive(Deserialize)]
struct Metric {
    name: String,
    unit: String,
    direction: String,
    #[serde(default)]
    aggregate: String,
}
#[derive(Deserialize)]
struct CaseResult {
    pass: bool,
    #[serde(default)]
    observed: String,
    #[serde(default)]
    metrics: BTreeMap<String, f64>,
}
#[derive(Deserialize, Serialize)]
struct Sample {
    #[serde(rename = "case")]
    id: String,
    #[serde(default)]
    revision: u32,
    number: u32,
    seed: i64,
    state: String,
    #[serde(default)]
    expected: Value,
    #[serde(default)]
    observed: String,
    #[serde(default)]
    values: BTreeMap<String, f64>,
    #[serde(default)]
    error: String,
}
impl Recipe {
    fn cases(&self) -> &[Case] {
        self.cases.as_deref().unwrap_or_default()
    }
    fn checks(&self) -> &[Vec<String>] {
        self.checks.as_deref().unwrap_or_default()
    }
}
fn recipe(value: &Value) -> Result<Recipe> {
    Ok(Recipe::deserialize(value)?)
}

pub fn validate(value: &Value) -> Result<()> {
    let r = recipe(value)?;
    ensure!(
        !r.checks().is_empty() || !r.cases().is_empty() || !r.rubric.trim().is_empty(),
        "supply at least one acceptance check"
    );
    for cmd in r.checks() {
        ensure!(
            cmd.first().is_some_and(|s| !s.trim().is_empty()),
            "every check needs an executable"
        );
    }
    ensure!(
        !r.preview
            .as_ref()
            .and_then(|v| v.first())
            .is_some_and(|s| s.trim().is_empty()),
        "preview needs an executable"
    );
    let mut ids = HashSet::new();
    for c in r.cases() {
        ensure!(
            !c.id.trim().is_empty()
                && ids.insert(&c.id)
                && c.samples <= 100
                && c.seed
                    .checked_add(i64::from(c.samples.saturating_sub(1)))
                    .is_some(),
            "invalid eval case ID or sample schedule"
        );
        ensure!(
            (Path::new(&c.driver).is_absolute() && c.program.is_empty())
                || (c.driver.is_empty()
                    && !c.interpreter.is_empty()
                    && !c.program.trim().is_empty()
                    && c.program.len() as u64 <= MAX_DRIVER),
            "case needs an absolute evaluator path or interpreted program"
        );
    }
    if let Some(rule) = &r.rule {
        ensure!(
            !r.cases().is_empty() && rule.minimum >= 0. && rule.min_lead >= 0.,
            "selection rules need cases and nonnegative thresholds"
        );
        let mut metrics = HashSet::new();
        for m in &rule.metrics {
            ensure!(
                !m.name.is_empty()
                    && !m.unit.is_empty()
                    && metrics.insert(m.name.as_str())
                    && matches!(m.direction.as_str(), "higher" | "lower")
                    && matches!(m.aggregate.as_str(), "" | "mean" | "median" | "p95"),
                "invalid metric"
            );
        }
        ensure!(
            metrics.contains(rule.objective.as_str())
                && (rule.tie_break.is_empty()
                    || (metrics.contains(rule.tie_break.as_str())
                        && rule.tie_break != rule.objective))
                && rule.limits.keys().all(|k| metrics.contains(k.as_str())),
            "invalid objective, limit or tie-break metric"
        );
    }
    Ok(())
}
fn which(program: &str) -> Option<PathBuf> {
    let executable = |p: &Path| {
        p.is_absolute()
            && p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let path = Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        return executable(path).then(|| path.to_owned());
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|p| p.join(program))
        .find(|p| executable(p))
}
pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    std::io::copy(&mut File::open(path)?, &mut digest)?;
    Ok(hex::encode(digest.finalize()))
}
fn verify(c: &Case) -> Result<()> {
    ensure!(
        !c.digest.is_empty() && hash_file(Path::new(&c.driver))? == c.digest,
        "approved evaluator changed"
    );
    Ok(())
}
pub fn pin(value: &Value, dir: &Path) -> Result<Value> {
    validate(value)?;
    let r = recipe(value)?;
    let mut pinned = value.clone();
    for (i, c) in r.cases().iter().enumerate() {
        let mut bytes = vec![];
        if c.driver.is_empty() {
            bytes.extend(c.program.as_bytes());
        } else {
            let f = File::open(&c.driver)?;
            ensure!(
                f.metadata()?.is_file(),
                "eval driver must be a regular file"
            );
            f.take(MAX_DRIVER + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_DRIVER,
                "eval driver exceeds 4 MiB"
            );
        }
        let digest = hex::encode(Sha256::digest(&bytes));
        fs::create_dir_all(dir)?;
        let path = dir.join(&digest);
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o500))?;
        if let Err(e) = tmp.persist_noclobber(&path) {
            ensure!(
                e.error.kind() == std::io::ErrorKind::AlreadyExists,
                "{}",
                e.error
            );
        }
        ensure!(hash_file(&path)? == digest, "approved evaluator changed");
        let obj = pinned["cases"][i]
            .as_object_mut()
            .context("case must be an object")?;
        obj.remove("program");
        obj.insert("driver".into(), json!(path));
        obj.insert("digest".into(), json!(digest));
        if !c.interpreter.is_empty() {
            obj.insert(
                "interpreter".into(),
                json!(which(&c.interpreter).context("eval interpreter unavailable")?),
            );
        }
    }
    Ok(pinned)
}
pub fn inputs(value: &Value, commit: &str) -> Result<String> {
    let r = recipe(value)?;
    let mut frozen = value.clone();
    let fields = frozen.as_object_mut().context("recipe must be an object")?;
    for key in ["rule", "preview", "rubric"] {
        fields.remove(key);
    }
    let mut binaries = BTreeMap::new();
    for program in r.checks().iter().filter_map(|a| a.first()).chain(
        r.cases()
            .iter()
            .map(|c| &c.interpreter)
            .filter(|s| !s.is_empty()),
    ) {
        if !binaries.contains_key(program) {
            binaries.insert(
                program,
                match which(program) {
                    Some(p) => format!("{}:{}", p.display(), hash_file(&p)?),
                    None => "from candidate or unavailable".into(),
                },
            );
        }
    }
    for arg in r.checks().iter().flat_map(|a| a.iter().skip(1)) {
        let p = Path::new(arg);
        if p.is_absolute() && p.is_file() {
            binaries.insert(arg, hash_file(p)?);
        }
    }
    for (i, c) in r.cases().iter().enumerate() {
        verify(c)?;
        frozen["cases"][i]
            .as_object_mut()
            .context("case must be an object")?
            .remove("driver");
    }
    crate::compat::inputs_digest(&frozen, commit, &json!(binaries))
}

pub fn command(dir: &Path, data: &Path, args: &[String], scope: &Value) -> Result<Command> {
    scoped_command(dir, data, args, scope, None)
}
fn scoped_command(
    dir: &Path,
    data: &Path,
    args: &[String],
    scope: &Value,
    driver: Option<&str>,
) -> Result<Command> {
    let (exe, rest) = args.split_first().context("missing executable")?;
    let mut cmd = if cfg!(target_os = "macos") {
        fn quoted(p: &Path) -> Result<String> {
            Ok(serde_json::to_string(
                &p.canonicalize()
                    .unwrap_or_else(|_| p.to_owned())
                    .to_str()
                    .context("invalid sandbox path")?,
            )?)
        }
        let outside = format!(
            "(require-not (subpath {})) (require-not (subpath {}))",
            quoted(dir)?,
            quoted(data)?
        );
        let mut policy = format!(
            "(version 1) (allow default) (deny file-write* (require-all {outside} (require-not (literal \"/dev/null\")))) "
        );
        let mut blocked: Vec<PathBuf> = ["repository", "state"]
            .iter()
            .filter_map(|k| scope[*k].as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        if let Some(home) = env::var_os("HOME") {
            for name in [".codex", ".claude", ".hermes", ".gemini"] {
                blocked.push(Path::new(&home).join(name));
            }
        }
        let except = match driver {
            Some(p) => format!("(require-not (literal {}))", quoted(Path::new(p))?),
            None => String::new(),
        };
        for p in blocked {
            policy += &format!(
                "(deny file-read-data (require-all (subpath {}) {outside} {except})) ",
                quoted(&p)?
            );
        }
        let mut cmd = Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-p").arg(policy).arg(exe);
        cmd
    } else {
        Command::new(exe)
    };
    cmd.args(rest)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::null());
    for k in ["PATH", "LANG", "LC_ALL", "TERM", "SystemRoot"] {
        if let Some(v) = env::var_os(k) {
            cmd.env(k, v);
        }
    }
    // Describe the already-applied OS profile to checks that start sandboxed subprocesses.
    if cfg!(target_os = "macos") {
        cmd.env("SWARM_EVAL_SANDBOX", "macos-seatbelt");
    }
    for k in ["HOME", "TMPDIR", "SWARM_DATA_DIR"] {
        cmd.env(k, data);
    }
    Ok(cmd)
}
// Each receipt is read while the program is running. Limit the output retained in memory.
// poll works even when a fast helper has already written its receipt and exited.
// On macOS, setting a socket timeout after the peer closes fails with EINVAL.
fn read_receipt(mut stream: std::os::unix::net::UnixStream, deadline: Instant) -> Result<Vec<u8>> {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::fd::AsFd;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("receipt timed out")?;
        let mut fds = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
        match poll(
            &mut fds,
            PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX),
        ) {
            Err(nix::errno::Errno::EINTR) | Ok(0) => continue,
            ready => {
                ready.context("poll receipt socket")?;
            }
        }
        let n = match stream.read(&mut buf) {
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            result => result.context("read receipt socket")?,
        };
        if n == 0 {
            return Ok(bytes);
        }
        ensure!(
            (bytes.len() + n) as u64 <= MAX_RECEIPT,
            "receipt exceeds 256 KiB"
        );
        bytes.extend_from_slice(&buf[..n]);
    }
}

pub(crate) fn execute(
    cmd: &mut Command,
    input: Option<&[u8]>,
    capture: bool,
    deadline: Instant,
) -> Result<Vec<u8>> {
    if let Some(input) = input {
        let mut f = tempfile::tempfile().context("create input file")?;
        f.write_all(input).context("write program input")?;
        f.rewind().context("rewind program input")?;
        cmd.stdin(f);
    }
    // A grandchild may inherit stdout after its parent exits. Keep the read
    // deadline effective even then, and never accumulate unbounded output.
    let socket = if capture {
        let (reader, writer) =
            std::os::unix::net::UnixStream::pair().context("open receipt socket")?;
        reader
            .set_nonblocking(true)
            .context("configure receipt reader")?;
        cmd.stdout(std::os::fd::OwnedFd::from(writer));
        Some(reader)
    } else {
        None
    };
    let mut child = cmd.spawn().context("launch approved program")?;
    // Command retains its configured descriptors for subsequent spawn calls.
    cmd.stdout(Stdio::null());
    let reader = socket.map(|stream| std::thread::spawn(move || read_receipt(stream, deadline)));
    let Some(status) = child
        .wait_timeout(deadline.saturating_duration_since(Instant::now()))
        .context("wait for approved program")?
    else {
        let _ = child.kill();
        let _ = child.wait();
        bail!("approved program timed out");
    };
    ensure!(status.success(), "approved program exited with {status}");
    match reader {
        Some(t) => t
            .join()
            .map_err(|_| anyhow::anyhow!("receipt reader failed"))?,
        None => Ok(vec![]),
    }
}
fn measure(
    c: &Case,
    n: u32,
    dir: &Path,
    data: &Path,
    scope: &Value,
    log: &File,
    deadline: Instant,
) -> Sample {
    let seed = c.seed + i64::from(n - 1);
    let mut sample = Sample {
        id: c.id.clone(),
        revision: c.revision,
        number: n,
        seed,
        state: "error".into(),
        observed: String::new(),
        error: String::new(),
        values: BTreeMap::new(),
        expected: c.expected.clone(),
    };
    let run = || -> Result<CaseResult> {
        verify(c)?;
        fs::create_dir_all(data)?;
        let args = [&c.interpreter, &c.driver]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let mut cmd = scoped_command(dir, data, &args, scope, Some(&c.driver))?;
        cmd.stderr(log.try_clone()?);
        let input = json!({"input":c.input,"expected":c.expected,"sample":n,"seed":seed});
        Ok(serde_json::from_slice(&execute(
            &mut cmd,
            Some(&serde_json::to_vec(&input)?),
            true,
            deadline,
        )?)?)
    };
    match run() {
        Ok(result) => {
            sample.state = if result.pass { "pass" } else { "failed" }.into();
            sample.observed = result.observed;
            sample.values = result.metrics;
        }
        Err(e) => sample.error = format!("{e:#}"),
    }
    sample
}
pub fn supervise(jobdir: &Path, job: &Value, log: &mut File) -> Result<()> {
    let value = &job["recipe"];
    validate(value)?;
    let recipe = recipe(value)?;
    let dir = job["request"]["dir"]
        .as_str()
        .context("missing eval directory")?;
    let commit = job["expected_commit"]
        .as_str()
        .context("missing eval commit")?;
    crate::project::check_snapshot(dir, commit)?;
    let data = jobdir.join("data");
    fs::create_dir_all(&data)?;
    let deadline = Instant::now() + TIMEOUT;
    let checks: Result<()> = (|| {
        for args in recipe.checks() {
            writeln!(log, "[check] {args:?}")?;
            let mut cmd = command(Path::new(dir), &data, args, &job["scope"])?;
            cmd.stdout(log.try_clone()?).stderr(log.try_clone()?);
            execute(&mut cmd, None, false, deadline)?;
        }
        Ok(())
    })();
    if !recipe.cases().is_empty() {
        ensure!(
            job["inputs"] == inputs(value, commit)?,
            "evaluation inputs changed before execution"
        );
        let deadline = Instant::now() + TIMEOUT;
        let mut samples = vec![];
        let mut failed = false;
        for (i, c) in recipe.cases().iter().enumerate() {
            for n in 1..=c.samples.max(1) {
                let sample = measure(
                    c,
                    n,
                    Path::new(dir),
                    &jobdir.join(format!("samples/case-{i}-{n}")),
                    &job["scope"],
                    log,
                    deadline,
                );
                writeln!(
                    log,
                    "[case] {} #{n} {} {}",
                    c.id, sample.state, sample.error
                )?;
                failed |= sample.state == "error" || (c.required && sample.state != "pass");
                samples.push(sample);
            }
        }
        let report = json!({"commit":commit,"inputs":job["inputs"],"samples":samples,"checks":if checks.is_ok(){"pass"}else{"failed"}});
        let mut tmp = tempfile::NamedTempFile::new_in(jobdir)?;
        serde_json::to_writer(&mut tmp, &report)?;
        tmp.as_file().sync_all()?;
        tmp.persist(jobdir.join("measurements.json"))?;
        ensure!(!failed, "one or more evaluator samples failed");
    }
    checks?;
    crate::project::check_snapshot(dir, commit)
}

pub(crate) fn summarize(value: &Value, report: &Value) -> Result<(String, String, Value)> {
    let r = recipe(value)?;
    let samples: Vec<Sample> = serde_json::from_value(report["samples"].clone())?;
    let inconclusive = |reason: &str| Ok(("inconclusive".into(), reason.into(), Value::Null));
    let mut failure = match report["checks"].as_str() {
        Some("pass") => String::new(),
        Some("failed") => "An acceptance check failed.".into(),
        _ => return inconclusive("Missing acceptance check result."),
    };
    let total = r
        .cases()
        .iter()
        .map(|c| c.samples.max(1) as usize)
        .sum::<usize>();
    if total != samples.len() {
        return inconclusive("The receipt is missing expected samples.");
    }
    let mut seen = HashSet::new();
    let held = r.cases().iter().any(|c| c.holdback);
    let mut measurements = BTreeMap::<&str, Vec<f64>>::new();
    for s in &samples {
        let Some(c) = r.cases().iter().find(|c| c.id == s.id) else {
            return inconclusive("Unknown case in receipt.");
        };
        if !seen.insert((&s.id, s.number))
            || s.revision != c.revision
            || s.number == 0
            || s.number > c.samples.max(1)
            || c.seed.checked_add(i64::from(s.number - 1)) != Some(s.seed)
        {
            return inconclusive("Unexpected sample schedule.");
        }
        match s.state.as_str() {
            "pass" => (),
            "failed" => {
                if c.required && failure.is_empty() {
                    failure = format!("Required case {} failed.", c.id);
                }
            }
            _ => return inconclusive("No usable sample result."),
        }
        if !held || c.holdback {
            for (k, v) in &s.values {
                measurements.entry(k).or_default().push(*v);
            }
        }
    }
    let mut scores = json!({});
    if let Some(rule) = r.rule {
        for m in rule.metrics {
            let Some(vals) = measurements.get_mut(m.name.as_str()) else {
                continue;
            };
            if vals.len() < rule.min_samples.max(1) as usize {
                continue;
            }
            vals.sort_by(f64::total_cmp);
            let n = vals.len();
            let v = match m.aggregate.as_str() {
                "median" if n % 2 == 0 => vals[n / 2] / 2. + vals[n / 2 - 1] / 2.,
                "median" => vals[n / 2],
                "p95" => vals[(95 * n).div_ceil(100) - 1],
                _ => vals.iter().map(|v| v / n as f64).sum(),
            };
            if v.is_finite() {
                scores[m.name] = json!({"value":v,"min":vals[0],"max":vals[n-1],"n":n});
            }
        }
    }
    Ok((
        if failure.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .into(),
        failure,
        scores,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_handles_closed_writer_and_enforces_one_deadline() -> Result<()> {
        let (stream, mut writer) = std::os::unix::net::UnixStream::pair()?;
        writer.write_all(b"final receipt")?;
        drop(writer);
        assert_eq!(
            read_receipt(stream, Instant::now() + Duration::from_secs(1))?,
            b"final receipt"
        );
        let (stream, mut held_writer) = std::os::unix::net::UnixStream::pair()?;
        held_writer.write_all(b"partial")?;
        let error = read_receipt(stream, Instant::now() + Duration::from_millis(30)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        Ok(())
    }
    #[test]
    fn requires_every_approved_sample() {
        let recipe =
            json!({"cases":[{"id":"rank","revision":2,"samples":2,"seed":10,"required":true}]});
        let first = json!({"case":"rank","revision":2,"number":1,"seed":10,"state":"pass"});
        let mut second = first.clone();
        second["number"] = json!(2);
        second["seed"] = json!(11);
        let mut wrong = second.clone();
        wrong["seed"] = json!(12);
        let mut failed = second.clone();
        failed["state"] = json!("failed");
        for (checks, samples, expected) in [
            ("pass", vec![&first, &second], "passed"),
            ("pass", vec![&first], "inconclusive"),
            ("pass", vec![&first, &first], "inconclusive"),
            ("pass", vec![&first, &wrong], "inconclusive"),
            ("", vec![&first, &second], "inconclusive"),
            ("pass", vec![&first, &failed], "failed"),
            ("failed", vec![&first, &second], "failed"),
        ] {
            assert_eq!(
                summarize(&recipe, &json!({"checks":checks,"samples":samples}))
                    .unwrap()
                    .0,
                expected
            );
        }
    }
    #[test]
    fn pinned_evaluator_is_immutable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("grader");
        fs::write(&source, b"old").unwrap();
        let r = json!({"cases":[{"id":"rank","driver":source}]});
        let pinned = pin(&r, &dir.path().join("approved")).unwrap();
        fs::write(&source, b"new").unwrap();
        let stable = inputs(&pinned, "123").unwrap();
        let mut other = pinned.clone();
        other["rubric"] = json!("another rubric");
        assert_eq!(inputs(&other, "123").unwrap(), stable);
        let file = Path::new(pinned["cases"][0]["driver"].as_str().unwrap());
        fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(file, b"changed").unwrap();
        assert!(inputs(&pinned, "123").is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn sandbox_writes_only_to_checkout_and_data() {
        let r = tempfile::tempdir().unwrap();
        let source = r.path().join("source");
        let data = r.path().join("data");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&data).unwrap();
        fs::write(r.path().join("token.json"), "private").unwrap();
        let helper = crate::test_examples::native_fixture(&source, "mock-sandbox").unwrap();
        let args = [helper.to_str().unwrap().to_owned()];
        let output = command(&source, &data, &args, &json!({"state":r.path()}))
            .unwrap()
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            source.join("inside").exists()
                && data.join("inside").exists()
                && !r.path().join("outside").exists()
        );
    }
}
