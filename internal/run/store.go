package run

import (
	"database/sql"
	"encoding/json"
	"errors"
	"maps"
	"os"
	"path/filepath"
	"slices"

	_ "modernc.org/sqlite"
)

type store struct {
	db *sql.DB
	// Accessed by the serialized scheduler and writers. Rebuilt after reopening.
	scheduled map[string]bool
}

func openStore(dir string) (*store, error) {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", filepath.Join(dir, "swarm.sqlite"))
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	var version int
	if err := db.QueryRow("PRAGMA user_version").Scan(&version); err != nil || version > 2 {
		_ = db.Close()
		if err != nil {
			return nil, err
		}
		return nil, errors.New("database format is newer than this Pantheon Local binary")
	}
	_, err = db.Exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
	if err == nil {
		err = migrate(db, version)
	}
	if err != nil {
		_ = db.Close()
		return nil, err
	}
	return &store{db: db}, nil
}

func migrate(db *sql.DB, version int) error {
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.Exec(`
		CREATE TABLE IF NOT EXISTS runs (id TEXT PRIMARY KEY, body BLOB NOT NULL);
		CREATE TABLE IF NOT EXISTS events (number INTEGER PRIMARY KEY, run_id TEXT NOT NULL, kind TEXT NOT NULL, body BLOB NOT NULL);
		CREATE TABLE IF NOT EXISTS projects (git_dir TEXT PRIMARY KEY, id TEXT NOT NULL UNIQUE);
		CREATE INDEX IF NOT EXISTS runs_by_project ON runs(json_extract(body,'$.project'));
		CREATE TABLE IF NOT EXISTS cases (project TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL, body BLOB NOT NULL, PRIMARY KEY(project,id,revision));
		PRAGMA user_version=2;`)
	if err != nil {
		return err
	}
	if version == 1 {
		if err := migrateQuestions(tx); err != nil {
			return err
		}
	}
	return tx.Commit()
}

func migrateQuestions(tx *sql.Tx) error {
	rows, err := tx.Query(`SELECT body FROM runs WHERE json_extract(body,'$.question') IS NOT NULL OR EXISTS
		(SELECT 1 FROM json_each(body,'$.tasks') WHERE json_extract(value,'$.state')='question')`)
	if err != nil {
		return err
	}
	defer rows.Close()
	var runs []Run
	for rows.Next() {
		var r Run
		var raw []byte
		if err := rows.Scan(&raw); err != nil {
			return err
		}
		if err := json.Unmarshal(raw, &r); err != nil {
			return err
		}
		if r.State == "question" && r.Question != "" {
			r.Inbox = append(r.Inbox, Question{ID: uid(), Text: r.Question, Source: "plan", State: "open", Revision: 1, Blocking: true})
		}
		for _, t := range r.Tasks {
			if t.State == "question" && t.Question != "" {
				r.Inbox = append(r.Inbox, Question{ID: uid(), Text: t.Question, Source: "legacy-worker", State: "open", Revision: 1, Tasks: []string{t.ID}, Blocking: true})
			}
		}
		if len(r.Inbox) > 0 {
			runs = append(runs, r)
		}
	}
	if err := rows.Err(); err != nil {
		return err
	}
	for _, r := range runs {
		r.Revision++
		raw, err := json.Marshal(r)
		if err != nil {
			return err
		}
		if _, err := tx.Exec("UPDATE runs SET body=? WHERE id=?", raw, r.ID); err != nil {
			return err
		}
	}
	return nil
}

func (s *store) projectID(gitDir string) (string, error) {
	if _, err := s.db.Exec("INSERT INTO projects VALUES (?,?) ON CONFLICT(git_dir) DO NOTHING", gitDir, uid()); err != nil {
		return "", err
	}
	var id string
	err := s.db.QueryRow("SELECT id FROM projects WHERE git_dir=?", gitDir).Scan(&id)
	return id, err
}

func (s *store) cases(project string, before int64, history bool) ([]EvalCase, error) {
	query := "SELECT rowid,body FROM cases WHERE project=? AND (?=0 OR rowid<?)"
	if !history {
		query += " AND revision=(SELECT max(revision) FROM cases latest WHERE latest.project=cases.project AND latest.id=cases.id)"
	}
	rows, err := s.db.Query(query+" ORDER BY rowid DESC LIMIT 100", project, before, before)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	list := []EvalCase{}
	for rows.Next() {
		var entry EvalCase
		var raw []byte
		var cursor int64
		if err := rows.Scan(&cursor, &raw); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(raw, &entry); err != nil {
			return nil, err
		}
		entry.Cursor = cursor
		list = append(list, entry)
	}
	return list, rows.Err()
}

func (s *store) caseRevision(project, id string, rev int) (EvalCase, error) {
	var entry EvalCase
	var raw []byte
	err := s.db.QueryRow("SELECT body FROM cases WHERE project=? AND id=? AND revision=?", project, id, rev).Scan(&raw)
	if err != nil {
		return entry, err
	}
	return entry, json.Unmarshal(raw, &entry)
}

func (s *store) save(r *Run, kind string, cases ...EvalCase) error {
	r.Revision++
	body, err := json.Marshal(r)
	if err != nil {
		return err
	}
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.Exec("INSERT INTO runs VALUES (?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body", r.ID, body); err != nil {
		return err
	}
	if _, err = tx.Exec("INSERT INTO events(run_id,kind,body) VALUES (?,?,?)", r.ID, kind, body); err != nil {
		return err
	}
	for _, entry := range cases {
		var revision int
		if err := tx.QueryRow("SELECT coalesce(max(revision),0) FROM cases WHERE project=? AND id=?", r.Project, entry.ID).Scan(&revision); err != nil {
			return err
		}
		if revision+1 != entry.Revision {
			return errors.New("eval case changed")
		}
		raw, err := json.Marshal(entry)
		if err != nil {
			return err
		}
		if _, err := tx.Exec("INSERT INTO cases VALUES (?,?,?,?)", r.Project, entry.ID, entry.Revision, raw); err != nil {
			return err
		}
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	if s.scheduled != nil {
		s.track(r)
	}
	return nil
}

func (s *store) track(r *Run) {
	if (r.State == "stopped" || r.State == "landed") && r.Landing == "" &&
		!slices.ContainsFunc(r.Turns, func(t Turn) bool { return t.State == "running" }) {
		switch r.State {
		case "stopped":
			delete(s.scheduled, r.ID)
			return
		case "landed":
			activeRelease := slices.ContainsFunc(r.Releases, func(v Release) bool {
				return !slices.Contains([]string{"deployed", "rolled-back", "failed", "superseded"}, v.State)
			})
			if !activeRelease && len(r.Observations) == 0 && pendingMessage(r) == nil {
				delete(s.scheduled, r.ID)
				return
			}
		}
	}
	s.scheduled[r.ID] = true
}

func (s *store) scheduledRuns() ([]Run, error) {
	if s.scheduled == nil {
		runs, err := s.list()
		if err != nil {
			return nil, err
		}
		s.scheduled = make(map[string]bool)
		for i := range runs {
			s.track(&runs[i])
		}
		return slices.DeleteFunc(runs, func(r Run) bool { return !s.scheduled[r.ID] }), nil
	}
	if len(s.scheduled) == 0 {
		return nil, nil
	}
	ids, err := json.Marshal(slices.Collect(maps.Keys(s.scheduled)))
	if err != nil {
		return nil, err
	}
	return readRuns(s.db.Query("SELECT body FROM runs WHERE id IN (SELECT value FROM json_each(?)) ORDER BY rowid DESC", string(ids)))
}

func (s *store) get(id string) (Run, error) {
	var raw []byte
	err := s.db.QueryRow("SELECT body FROM runs WHERE id=?", id).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		return Run{}, errors.New("run not found")
	}
	if err != nil {
		return Run{}, err
	}
	var r Run
	err = json.Unmarshal(raw, &r)
	return r, err
}

func (s *store) list() ([]Run, error) {
	return readRuns(s.db.Query("SELECT body FROM runs ORDER BY rowid DESC"))
}

// Snapshot reads the runs and the last event visible in the same SQLite snapshot.
func (e *Engine) Snapshot() (Snapshot, error) {
	var result Snapshot
	tx, err := e.db.db.Begin()
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	if err := tx.QueryRow("SELECT coalesce(max(number),0) FROM events").Scan(&result.Cursor); err != nil {
		return result, err
	}
	result.Runs, err = readRuns(tx.Query("SELECT body FROM runs ORDER BY rowid DESC"))
	if err != nil {
		return result, err
	}
	return result, tx.Commit()
}

type Snapshot struct {
	Cursor int64 `json:"cursor"`
	Runs   []Run `json:"runs"`
}

func readRuns(rows *sql.Rows, err error) ([]Run, error) {
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	list := []Run{}
	for rows.Next() {
		var raw []byte
		var r Run
		if err := rows.Scan(&raw); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(raw, &r); err != nil {
			return nil, err
		}
		list = append(list, r)
	}
	return list, rows.Err()
}

func (s *store) events(after int64) ([]Update, error) {
	rows, err := s.db.Query("SELECT number, kind, body FROM events WHERE number > ? ORDER BY number LIMIT 100", after)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	list := []Update{}
	for rows.Next() {
		var u Update
		var raw []byte
		if err := rows.Scan(&u.Number, &u.Kind, &raw); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(raw, &u.Run); err != nil {
			return nil, err
		}
		list = append(list, u)
	}
	return list, rows.Err()
}

type RunSummary struct {
	Cursor int64  `json:"cursor"`
	ID     string `json:"id"`
	Brief  string `json:"brief"`
	State  string `json:"state"`
	Round  int    `json:"round"`
}

func (s *store) projectHistory(project string, before int64) ([]RunSummary, error) {
	rows, err := s.db.Query(`SELECT rowid,id,json_extract(body,'$.input.prompt'),json_extract(body,'$.state'),coalesce(json_extract(body,'$.round'),0)
		FROM runs WHERE json_extract(body,'$.project')=? AND (?=0 OR rowid<?) ORDER BY rowid DESC LIMIT 20`, project, before, before)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	history := []RunSummary{}
	for rows.Next() {
		var item RunSummary
		if err := rows.Scan(&item.Cursor, &item.ID, &item.Brief, &item.State, &item.Round); err != nil {
			return nil, err
		}
		history = append(history, item)
	}
	return history, rows.Err()
}
