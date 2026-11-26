use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Params, params};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub struct Store(Mutex<Database>);
struct Database {
    conn: Connection,
    scheduled: HashSet<String>,
}

fn read_values(conn: &Connection, query: &str, args: impl Params) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map(args, |r| r.get::<_, Vec<u8>>(0))?;
    rows.map(|row| Ok(serde_json::from_slice(&row?)?)).collect()
}

fn scheduled(run: &Value) -> bool {
    let any = |field: &str, pred: fn(&Value) -> bool| {
        run[field].as_array().is_some_and(|v| v.iter().any(pred))
    };
    if any("turns", |t| t["state"] == "running")
        || run["landing"].as_str().is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    let over_budget = run["input"]["token_target"]
        .as_i64()
        .is_some_and(|limit| limit > 0 && run["tokens"].as_i64().unwrap_or(0) >= limit);
    let can_observe = !over_budget && run["observer_error"].as_str().is_none_or(str::is_empty);
    match run["state"].as_str() {
        Some("stopped") => false,
        Some("landed") => {
            any("releases", |r| {
                !matches!(
                    r["state"].as_str(),
                    Some("deployed" | "rolled-back" | "failed" | "superseded" | "cancelled")
                )
            }) || (can_observe && any("observations", |_| true))
                || any("conversation", |m| m["state"] == "queued")
        }
        _ => true,
    }
}
impl Store {
    fn lock(&self) -> Result<MutexGuard<'_, Database>> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))
    }
    pub fn open(dir: &Path) -> Result<Self> {
        let mut conn = Connection::open(dir.join("swarm.sqlite"))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 2,
            "database format is newer than this Pantheon Local binary"
        );
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS runs (id TEXT PRIMARY KEY, body BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS events (number INTEGER PRIMARY KEY, run_id TEXT NOT NULL, kind TEXT NOT NULL, body BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS projects (git_dir TEXT PRIMARY KEY,id TEXT NOT NULL UNIQUE);
        CREATE INDEX IF NOT EXISTS runs_by_project ON runs(json_extract(body,'$.project'));
        CREATE TABLE IF NOT EXISTS cases (project TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,body BLOB NOT NULL,PRIMARY KEY(project,id,revision));
        PRAGMA user_version=2;")?;
        let mut ids = HashSet::new();
        let mut history = Vec::new();
        for mut r in read_values(&tx, "SELECT body FROM runs", [])? {
            if version == 1 {
                let mut inbox = r["inbox"].as_array().cloned().unwrap_or_default();
                if r["state"] == "question" && r["question"].as_str().is_some_and(|s| !s.is_empty())
                {
                    inbox.push(json!({"id":crate::uid(),"text":r["question"],"source":"plan","state":"open","revision":1,"blocking":true}));
                }
                for t in r["tasks"].as_array().into_iter().flatten().filter(|t| {
                    t["state"] == "question"
                        && t["question"].as_str().is_some_and(|s| !s.is_empty())
                }) {
                    inbox.push(json!({"id":crate::uid(),"text":t["question"],"source":"legacy-worker","tasks":[t["id"]],"state":"open","revision":1,"blocking":true}));
                }
                if !inbox.is_empty() {
                    r["inbox"] = inbox.into();
                    r["revision"] = (r["revision"].as_i64().unwrap_or(0) + 1).into();
                    tx.execute(
                        "UPDATE runs SET body=? WHERE id=?",
                        params![serde_json::to_vec(&r)?, r["id"].as_str()],
                    )?;
                }
            }
            if scheduled(&r) {
                ids.insert(r["id"].as_str().context("missing run ID")?.to_owned());
            }
            if crate::run::failure_log::index_completed(&mut r) {
                history.push(r);
            }
        }
        tx.commit()?;
        let store = Self(Mutex::new(Database {
            conn,
            scheduled: ids,
        }));
        for mut run in history {
            store.save(&mut run, "indexed-recovery-history", &[])?;
        }
        Ok(store)
    }
    pub fn project_id(&self, git_dir: &str) -> Result<String> {
        let db = self.lock()?;
        db.conn.execute(
            "INSERT INTO projects VALUES (?,?) ON CONFLICT(git_dir) DO NOTHING",
            params![git_dir, crate::uid()],
        )?;
        Ok(db
            .conn
            .query_row("SELECT id FROM projects WHERE git_dir=?", [git_dir], |r| {
                r.get(0)
            })?)
    }
    pub fn get(&self, id: &str) -> Result<Value> {
        read_values(&self.lock()?.conn, "SELECT body FROM runs WHERE id=?", [id])?
            .pop()
            .context("run not found")
    }
    pub fn list(&self) -> Result<Vec<Value>> {
        read_values(
            &self.lock()?.conn,
            "SELECT body FROM runs ORDER BY rowid DESC",
            [],
        )
    }
    pub fn deleted_runs(&self) -> Result<Vec<Value>> {
        read_values(
            &self.lock()?.conn,
            "SELECT body FROM events WHERE kind='run-deleted'",
            [],
        )
    }

    pub fn delete(&self, id: &str, expected: i64) -> Result<Value> {
        let mut db = self.lock()?;
        let tx = db.conn.transaction()?;
        let run = read_values(&tx, "SELECT body FROM runs WHERE id=?", [id])?.pop();
        let Some(run) = run else {
            return read_values(
                &tx,
                "SELECT body FROM events WHERE run_id=? AND kind='run-deleted'",
                [id],
            )?
            .pop()
            .context("run not found");
        };
        ensure!(
            run["revision"] == expected,
            "the run changed; review its current state and retry"
        );
        ensure!(
            matches!(run["state"].as_str(), Some("landed" | "stopped")),
            "pause the run before deleting it"
        );
        ensure!(
            !run["turns"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| t["state"] == "running")
                && run["landing"].as_str().is_none_or(str::is_empty),
            "wait for active work to finish before deleting this run"
        );
        ensure!(
            run["releases"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|r| matches!(
                    r["state"].as_str(),
                    Some("deployed" | "rolled-back" | "failed" | "superseded" | "cancelled")
                )),
            "finish or reconcile the release before deleting this run"
        );
        let deleted = json!({"id":id,"deleted":true});
        tx.execute(
            "INSERT INTO events(run_id,kind,body) VALUES (?,'run-deleted',?)",
            params![id, serde_json::to_vec(&deleted)?],
        )?;
        tx.execute(
            "DELETE FROM events WHERE run_id=? AND number<>?",
            params![id, tx.last_insert_rowid()],
        )?;
        tx.execute("DELETE FROM runs WHERE id=?", [id])?;
        tx.commit()?;
        db.scheduled.remove(id);
        Ok(deleted)
    }

    pub fn scheduled_runs(&self) -> Result<Vec<Value>> {
        let db = self.lock()?;
        if db.scheduled.is_empty() {
            return Ok(vec![]);
        }
        read_values(
            &db.conn,
            "SELECT body FROM runs WHERE id IN (SELECT value FROM json_each(?)) ORDER BY rowid DESC",
            [serde_json::to_string(&db.scheduled)?],
        )
    }
    pub fn save(&self, r: &mut Value, kind: &str, cases: &[Value]) -> Result<()> {
        let mut db = self.lock()?;
        let tx = db.conn.transaction()?;
        let revision = r["revision"]
            .as_i64()
            .unwrap_or(0)
            .checked_add(1)
            .context("run revision overflow")?;
        // Serialize borrowed fields so large conversations and candidate histories
        // don't need a second in-memory copy for each scheduler update.
        let mut body = Vec::new();
        {
            use serde::ser::{SerializeMap, Serializer};
            let mut serializer = serde_json::Serializer::new(&mut body);
            let fields = r.as_object().context("invalid run")?;
            let mut map = serializer.serialize_map(None)?;
            for (key, value) in fields.iter().filter(|(key, _)| key.as_str() != "revision") {
                map.serialize_entry(key, value)?;
            }
            map.serialize_entry("revision", &revision)?;
            map.end()?;
        }
        let id = r["id"].as_str().context("missing run ID")?;
        tx.execute(
            "INSERT INTO runs VALUES (?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            params![id, body],
        )?;
        tx.execute(
            "INSERT INTO events(run_id,kind,body) VALUES (?,?,?)",
            params![id, kind, body],
        )?;
        tx.execute(
            "DELETE FROM events WHERE run_id=? AND number<>?",
            params![id, tx.last_insert_rowid()],
        )?;
        for entry in cases {
            let project = r["project"].as_str().context("missing project")?;
            let case = entry["id"].as_str().context("missing case ID")?;
            let revision = entry["revision"]
                .as_i64()
                .context("missing case revision")?;
            let last: i64 = tx.query_row(
                "SELECT coalesce(max(revision),0) FROM cases WHERE project=? AND id=?",
                params![project, case],
                |r| r.get(0),
            )?;
            ensure!(revision == last + 1, "eval case changed");
            tx.execute(
                "INSERT INTO cases VALUES (?,?,?,?)",
                params![project, case, revision, serde_json::to_vec(entry)?],
            )?;
        }
        tx.commit()?;
        if scheduled(r) {
            db.scheduled.insert(id.into());
        } else {
            db.scheduled.remove(id);
        }
        r["revision"] = revision.into();
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Value> {
        let mut db = self.lock()?;
        let tx = db.conn.transaction()?;
        let cursor: i64 = tx.query_row("SELECT coalesce(max(number),0) FROM events", [], |r| {
            r.get(0)
        })?;
        let runs = read_values(&tx, "SELECT body FROM runs ORDER BY rowid DESC", [])?;
        tx.commit()?;
        Ok(json!({ "runs":runs,"cursor":cursor }))
    }
    pub fn events(&self, after: i64) -> Result<Vec<Value>> {
        let db = self.lock()?;
        let mut st = db.conn.prepare(
            "SELECT number,kind,body FROM events WHERE number>? ORDER BY number LIMIT 100",
        )?;
        let mut rows = st.query([after])?;
        let mut result = vec![];
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(2)?;
            result.push(json!({"number":row.get::<_,i64>(0)?,"kind":row.get::<_,String>(1)?,"run":serde_json::from_slice::<Value>(&raw)?}));
        }
        Ok(result)
    }
    pub fn cases(&self, project: &str, before: i64, history: bool) -> Result<Vec<Value>> {
        let db = self.lock()?;
        let mut st=db.conn.prepare("SELECT rowid,body FROM cases WHERE project=?1 AND (?2=0 OR rowid<?2) AND (?3 OR revision=(SELECT max(revision) FROM cases latest WHERE latest.project=cases.project AND latest.id=cases.id)) ORDER BY rowid DESC LIMIT 100")?;
        let mut rows = st.query(params![project, before, history])?;
        let mut list = vec![];
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(1)?;
            let mut v: Value = serde_json::from_slice(&raw)?;
            v["cursor"] = row.get::<_, i64>(0)?.into();
            list.push(v);
        }
        Ok(list)
    }
    pub fn case_revision(&self, project: &str, id: &str, rev: i64) -> Result<Value> {
        let raw: Vec<u8> = self
            .lock()?
            .conn
            .query_row(
                "SELECT body FROM cases WHERE project=? AND id=? AND revision=?",
                params![project, id, rev],
                |r| r.get(0),
            )
            .optional()?
            .context("case revision not found")?;
        Ok(serde_json::from_slice(&raw)?)
    }
    pub fn project_history(&self, project: &str, before: i64) -> Result<Vec<Value>> {
        let db = self.lock()?;
        let mut st=db.conn.prepare("SELECT rowid,id,json_extract(body,'$.input.prompt'),json_extract(body,'$.state'),coalesce(json_extract(body,'$.round'),0),json_extract(body,'$.title') FROM runs WHERE json_extract(body,'$.project')=?1 AND (?2=0 OR rowid<?2) ORDER BY rowid DESC LIMIT 20")?;
        Ok(st.query_map(params![project,before],|r|Ok(json!({"cursor":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"brief":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"round":r.get::<_,i64>(4)?,"title":r.get::<_,Option<String>>(5)?})))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delete_purges_one_runs_history_and_preserves_the_event_cursor() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Store::open(dir.path())?;
        let mut run = json!({"id":"first","state":"working","input":{"prompt":"private brief"}});
        let mut other = json!({"id":"other","state":"working"});
        db.save(&mut run, "started", &[])?;
        db.save(&mut other, "started", &[])?;
        assert!(db.delete("first", 1).is_err());
        run["state"] = json!("stopped");
        run["turns"] = json!([{"state":"running"}]);
        db.save(&mut run, "stopping", &[])?;
        assert!(db.delete("first", 2).is_err());
        run["turns"] = json!([]);
        run["releases"] = json!([{"state":"uncertain"}]);
        db.save(&mut run, "held", &[])?;
        assert!(db.delete("first", 3).is_err());
        run["releases"] = json!([]);
        db.save(&mut run, "finished", &[])?;
        assert!(db.delete("first", 3).is_err());
        let cursor = db.snapshot()?["cursor"].as_i64().unwrap();
        let deleted = db.delete("first", 4)?;
        assert_eq!(db.delete("first", 4)?, deleted);
        assert_eq!(db.list()?, vec![other.clone()]);
        assert_eq!(db.scheduled_runs()?, vec![other]);
        assert_eq!(db.events(cursor)?[0]["run"], deleted);
        assert!(!serde_json::to_string(&db.events(0)?)?.contains("private brief"));
        drop(db);
        assert_eq!(Store::open(dir.path())?.deleted_runs()?, vec![deleted]);
        Ok(())
    }

    #[test]
    fn save_rolls_back_event_and_run_on_case_conflict() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Store::open(dir.path())?;
        let mut r = json!({"id":"team","project":"p","state":"stopped"});
        let entry = json!({"id":"check","revision":1});
        db.save(&mut r, "started", std::slice::from_ref(&entry))?;
        assert!(db.save(&mut r, "changed", &[entry]).is_err());
        assert_eq!(r["revision"], 1);
        assert_eq!(db.snapshot()?["cursor"], 1);
        assert!(db.scheduled_runs()?.is_empty());
        drop(db);
        assert_eq!(Store::open(dir.path())?.get("team")?, r);
        Ok(())
    }

    #[test]
    fn save_compacts_superseded_run_events_and_skips_blocked_landed_observers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Store::open(dir.path())?;
        let mut r = json!({"id":"r1","state":"landed","tokens":10,"input":{"token_target":10},"observations":[{"id":"o1"}]});
        db.save(&mut r, "first", &[])?;
        assert!(db.scheduled_runs()?.is_empty());
        r["detail"] = json!("second");
        db.save(&mut r, "second", &[])?;
        let events = db.events(0)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["number"], 2);
        assert_eq!(events[0]["run"]["detail"], "second");
        Ok(())
    }
}

#[cfg(test)]
mod upgrades {
    use super::*;
    #[test]
    fn version_one_questions_survive_restart_without_changing_delivery_authority() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("swarm.sqlite");
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE runs(id TEXT PRIMARY KEY,body BLOB NOT NULL);PRAGMA user_version=1;",
        )?;
        let old = json!({"id":"old","state":"question","question":"Who can view the export?","input":{"recipe":{"checks":[["true"]]}},"tasks":[{"state":"question","id":"ui","question":"Which view?"}]});
        conn.execute(
            "INSERT INTO runs VALUES (?,?)",
            params!["old", serde_json::to_vec(&old)?],
        )?;
        drop(conn);
        let db = Store::open(dir.path())?;
        let run = db.get("old")?;
        let inbox = run["inbox"].as_array().unwrap();
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0]["source"], "plan");
        assert_eq!(inbox[1]["tasks"], json!(["ui"]));
        assert!(run["input"]["delivery"].is_null());
        drop(db);
        assert_eq!(Store::open(dir.path())?.get("old")?, run);
        Ok(())
    }
    #[test]
    fn future_state_format_is_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("swarm.sqlite");
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA user_version=3")?;
        drop(conn);
        assert!(Store::open(dir.path()).is_err());
        let version: i64 =
            Connection::open(path)?.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(version, 3);
        Ok(())
    }
}
