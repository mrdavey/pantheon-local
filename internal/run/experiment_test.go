package run

import (
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/project"
)

func TestMigratePendingV1Run(t *testing.T) {
	dir := t.TempDir()
	db, err := sql.Open("sqlite", filepath.Join(dir, "swarm.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = db.Exec(`CREATE TABLE runs (id TEXT PRIMARY KEY, body BLOB NOT NULL);
CREATE TABLE events (number INTEGER PRIMARY KEY, run_id TEXT NOT NULL, kind TEXT NOT NULL, body BLOB NOT NULL);
PRAGMA user_version=1;`)
	if err != nil {
		t.Fatal(err)
	}
	raw := []byte(`{"id":"old","state":"question","question":"Who can view quotes?","input":{"recipe":{"checks":[["true"]]}}}`)
	if _, err := db.Exec("INSERT INTO runs VALUES (?,?)", "old", raw); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	e, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	r, err := e.Get("old")
	if err != nil || r.Question != "Who can view quotes?" || len(r.Inbox) != 1 {
		t.Fatalf("v1 question was lost: %+v %v", r, err)
	}
	r, err = e.Command(t.Context(), r.ID, Command{Kind: "answer-inbox", Task: r.Inbox[0].ID, Text: "Only me"})
	if err != nil || r.State != "planning" || r.Answer != "Only me" || r.Input.Delivery != "" {
		t.Fatalf("v1 action failed or gained authority: %+v %v", r, err)
	}
	var version int
	if err := e.db.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil || version != 2 {
		t.Fatalf("schema version %d: %v", version, err)
	}
}

func TestObservationValidationAndCoordinatorSwitch(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	r := Run{ID: "run", Project: "project", State: "working", Generation: 1, Input: Start{Coordinator: "first"},
		Profiles: map[string]adapter.Profile{"second": {Kind: "custom"}}, Repo: project.Repository{Common: "/repo/git"}}
	r.observe("feedback", "", "Users expect exact matches first")
	if err := e.db.save(&r, "started"); err != nil {
		t.Fatal(err)
	}
	observation := r.Observations[0]
	reply := struct{ Actions []coordinatorAction }{[]coordinatorAction{
		{ID: "record-case", Kind: "save_case_revision", Ref: observation.ID, Case: &EvalCase{
			ID: "exact-match", Revision: 1, Scenario: "Find a titled note", Expected: "Exact match first", Why: "Regression report", State: "draft"}},
		{ID: "broken", Kind: "ask", Ref: "stale", Question: &Question{Text: "What result matters?"}},
	}}
	body, _ := json.Marshal(reply)
	turn := Turn{Task: observation.ID, Generation: 1}
	if err := e.finishObservation(&r, turn, adapter.Result{Text: string(body)}); err != nil {
		t.Fatal(err)
	}
	if len(r.Observations) != 1 || r.ObserverError == "" {
		t.Fatalf("invalid batch consumed evidence: %+v", r)
	}
	if cases, _ := e.Cases(r.ID, 0); len(cases) != 0 {
		t.Fatalf("invalid batch saved partial cases: %+v", cases)
	}
	r, err = e.Command(t.Context(), r.ID, Command{Kind: "coordinator", Text: "second"})
	if err != nil || r.Generation != 2 || r.ObserverError != "" {
		t.Fatalf("coordinator switch failed: %+v %v", r, err)
	}
	reply.Actions = reply.Actions[:1]
	body, _ = json.Marshal(reply)
	if err := e.finishObservation(&r, turn, adapter.Result{Text: string(body)}); err != nil || len(r.Observations) != 1 {
		t.Fatalf("old coordinator consumed the observation: %+v %v", r, err)
	}
	turn.Generation = r.Generation
	if err := e.finishObservation(&r, turn, adapter.Result{Text: string(body)}); err != nil {
		t.Fatal(err)
	}
	if err := e.finishObservation(&r, turn, adapter.Result{Text: string(body)}); err != nil {
		t.Fatal(err)
	}
	cases, err := e.Cases(r.ID, 0)
	if err != nil || len(cases) != 1 || cases[0].Author != "second" || len(r.Observations) != 0 {
		t.Fatalf("expected exactly one case from the new coordinator: %+v %v", cases, err)
	}
}

func TestVariantMembershipAndComparison(t *testing.T) {
	tasks := []Task{
		{ID: "shared", Goal: "Common API", Worker: "worker"},
		{ID: "a-api", Goal: "Claim output", Worker: "worker", Variant: "a"},
		{ID: "a-ui", Goal: "Display output", Worker: "worker", Variant: "a", Depends: []string{"a-api"}},
		{ID: "b-api", Goal: "Use other algorithm", Worker: "worker", Variant: "b"},
	}
	if err := validateTasks(tasks, nil, []string{"worker"}); err != nil {
		t.Fatal(err)
	}
	sets, err := selections(tasks)
	if err != nil || len(sets) != 2 || len(sets[0]) != 3 || len(sets[1]) != 2 {
		t.Fatalf("selections = %v, %v", sets, err)
	}
	tasks[2].Depends = []string{"b-api"}
	if validateTasks(tasks, nil, []string{"worker"}) == nil {
		t.Fatal("cross-variant dependency passed")
	}
	rule := &eval.Rule{Objective: "score", MinLead: 2, Metrics: []eval.Metric{{Name: "score", Direction: "higher", Unit: "count"}}}
	r := Run{Input: Start{Recipe: eval.Recipe{Rule: rule}}, Candidates: []Candidate{
		{ID: "a", State: "passed", Score: map[string]eval.Summary{"score": {Value: 9, N: 3}}},
		{ID: "b", State: "passed", Score: map[string]eval.Summary{"score": {Value: 8, N: 3}}},
	}}
	if choice := recommend(&r); choice.State != "undecided" {
		t.Fatalf("close race recommended a winner: %+v", choice)
	}
	r.Candidates[1].State = "inconclusive"
	if choice := recommend(&r); choice.State != "undecided" {
		t.Fatalf("incomplete result recommended a winner: %+v", choice)
	}
}

func TestBlindReviewHidesPairUntilVote(t *testing.T) {
	e := Engine{}
	r := Run{State: "review", Candidates: []Candidate{{ID: "a", Commit: "a"}, {ID: "b", Commit: "b"}},
		Input: Start{Recipe: eval.Recipe{Rubric: "Which preview is clearer?"}}}
	if err := e.preference(&r, Command{Kind: "blind-review", Candidate: "a", Donors: []string{"b"}}); err != nil {
		t.Fatal(err)
	}
	private, public := r.Preferences[0], r.Public().Preferences[0]
	if private.Left == "" || private.Right == "" || public.Left != "" || public.Right != "" {
		t.Fatalf("private pair %+v leaked to %+v", private, public)
	}
	if err := e.preference(&r, Command{Kind: "vote", Task: private.ID, Text: "tie"}); err != nil {
		t.Fatal(err)
	}
	answer := r.Public().Preferences[0]
	if answer.Left != private.Left || answer.Choice != "tie" {
		t.Fatalf("recorded vote not available: %+v", answer)
	}
}

func TestEvaluationRequiresEveryApprovedSample(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	sample := eval.Sample{Case: "rank", Revision: 2, Number: 1, Seed: 10, State: "pass"}
	second := sample
	second.Number = 2
	second.Seed = 11
	wrong := second
	wrong.Seed = 12
	failed := second
	failed.State = "failed"
	for _, tc := range []struct {
		name, checks string
		samples      []eval.Sample
		state        string
	}{
		{"complete", "pass", []eval.Sample{sample, second}, "passed"},
		{"missing", "pass", []eval.Sample{sample}, "inconclusive"},
		{"duplicate", "pass", []eval.Sample{sample, sample}, "inconclusive"},
		{"wrong seed", "pass", []eval.Sample{sample, wrong}, "inconclusive"},
		{"missing check receipt", "", []eval.Sample{sample, second}, "inconclusive"},
		{"required case failure", "pass", []eval.Sample{sample, failed}, "failed"},
		{"failed check", "failed", []eval.Sample{sample, second}, "failed"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := Run{ID: "r", Input: Start{Recipe: eval.Recipe{Cases: []eval.Case{{ID: "rank", Revision: 2, Samples: 2, Seed: 10, Required: true}}}},
				Candidates: []Candidate{{ID: "candidate", Inputs: "inputs", Commit: "abc"}}}
			id := uid()
			dir := e.jobdir(r, id)
			if err := os.MkdirAll(dir, 0700); err != nil {
				t.Fatal(err)
			}
			if err := writeJSON(filepath.Join(dir, "measurements.json"), eval.Report{Commit: "abc", Inputs: "inputs", Checks: tc.checks, Samples: tc.samples}); err != nil {
				t.Fatal(err)
			}
			if err := e.evalResult(&r, Turn{ID: id, Task: "candidate"}, adapter.Result{}); err != nil {
				t.Fatal(err)
			}
			c := r.Candidates[0]
			if c.State != tc.state || c.Evidence != id {
				t.Fatalf("candidate = %+v, want %s with evidence", c, tc.state)
			}
		})
	}
}

func TestDelegationInheritsParentScopeAndInputs(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	for _, tc := range []struct {
		name, state, path string
		accept            bool
	}{
		{"inherit", "question", "", true}, {"narrow", "question", "src/api", true}, {"sibling path", "question", "src-other", false}, {"closed parent", "committed", "src", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := Run{ID: uid(), Project: "project", State: "working", Input: Start{Workers: []string{"w"}}, Tasks: []Task{
				{ID: "base", Worker: "w", State: "committed"}, {ID: "parent", State: tc.state, Worker: "w", Paths: []string{"src"}, Variant: "balanced", Depends: []string{"base"}},
			}}
			r.observe("worker-question", "parent", "Split the implementation?")
			task := Task{ID: "child", Goal: "Implement the API", Worker: "w"}
			if tc.path != "" {
				task.Paths = []string{tc.path}
			}
			body, err := json.Marshal(map[string]any{"actions": []coordinatorAction{{ID: "hand-off", Kind: "delegate", Ref: "parent", Tasks: []Task{task}}}})
			if err != nil {
				t.Fatal(err)
			}
			if err := e.finishObservation(&r, Turn{Task: r.Observations[0].ID}, adapter.Result{Text: string(body)}); err != nil {
				t.Fatal(err)
			}
			if !tc.accept {
				if len(r.Tasks) != 2 || len(r.Observations) != 1 || r.ObserverError == "" {
					t.Fatalf("invalid handoff accepted: %+v", r)
				}
				return
			}
			child := taskByID(&r, "child")
			want := tc.path
			if want == "" {
				want = "src"
			}
			if child == nil || child.Parent != "parent" || child.Variant != "balanced" || len(child.Paths) != 1 || child.Paths[0] != want ||
				len(child.Depends) != 1 || child.Depends[0] != "base" {
				t.Fatalf("child did not inherit boundaries: %+v", child)
			}
		})
	}
}

func TestParentWaitsForChildrenWithinItsAlternative(t *testing.T) {
	r := Run{Tasks: []Task{
		{ID: "a", Alternative: "approach"}, {ID: "b", Alternative: "approach"},
		{ID: "child", Parent: "a", State: "committed"}, {ID: "sibling", Parent: "a", State: "running"},
	}}
	if depsReady(&r, &r.Tasks[0]) || !depsReady(&r, &r.Tasks[1]) {
		t.Fatal("outstanding child should delay only its parent")
	}
	grandchild := Task{ID: "next", Depends: []string{"a"}}
	if !dependsOn(&r, []Task{grandchild}, "next", "sibling") {
		t.Fatal("grandchild waiting for its ancestor creates a cycle")
	}
	r.Tasks[3].State = "committed"
	if !depsReady(&r, &r.Tasks[0]) {
		t.Fatal("parent did not wake after both children completed")
	}
	sets, err := selections(r.Tasks)
	if err != nil || len(sets) != 2 || len(sets[0]) != 3 || len(sets[1]) != 1 || sets[1][0] != "b" {
		t.Fatalf("child commits crossed alternatives: %v %v", sets, err)
	}
}
