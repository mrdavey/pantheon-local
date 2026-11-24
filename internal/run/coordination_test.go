package run

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
)

func TestTurnContextSharesApprovedCriteriaAndPeerScope(t *testing.T) {
	r := Run{RecipeApproved: true, Answer: "legacy answer", Input: Start{Recipe: eval.Recipe{
		Checks: [][]string{{"check-contract"}}, Rubric: "readable", Rule: &eval.Rule{Objective: "latency"},
		Cases: []eval.Case{{Holdback: true, Expected: json.RawMessage(`"private expectation"`), Driver: "/private/driver"}},
	}}, Tasks: []Task{
		{ID: "self", Variant: "first", State: "running"}, {ID: "my-peer", Variant: "first", State: "pending"},
		{ID: "other-candidate", Variant: "second", State: "running"}, {ID: "finished", Variant: "first", State: "committed"},
	}}
	raw, err := r.turnContext(&r.Tasks[0])
	if err != nil {
		t.Fatal(err)
	}
	for _, include := range []string{"legacy answer", "readable", "latency", "check-contract", "my-peer"} {
		if !strings.Contains(string(raw), include) {
			t.Fatalf("missing %q from %s", include, raw)
		}
	}
	for _, exclude := range []string{"private", "other-candidate", "finished"} {
		if strings.Contains(string(raw), exclude) {
			t.Fatalf("unexpected %q in %s", exclude, raw)
		}
	}
	r.RecipeApproved = false
	raw, err = r.turnContext(nil)
	if err != nil || strings.Contains(string(raw), "latency") || strings.Contains(string(raw), "check-contract") {
		t.Fatalf("unapproved criteria in %s: %v", raw, err)
	}
}

func TestRoutineCompletionsCanShareCoordinatorTurn(t *testing.T) {
	r := Run{State: "working", Project: "team"}
	for _, id := range []string{"api", "docs", "cli"} {
		r.observe("worker-completed", id, "Checks passed.")
	}
	if r.observationsReady(true) || !r.observationsReady(false) {
		t.Fatal("expected one review after the remaining workers finish")
	}
	for _, kind := range []string{"worker-question", "worker-finding", "developer-answer"} {
		r.observe(kind, "ui", "A decision is ready.")
		if !r.observationsReady(true) {
			t.Fatalf("routine completions delayed %s", kind)
		}
		r.Observations = r.Observations[:len(r.Observations)-1]
	}
	for len(r.Observations) < observationBatchSize {
		r.observe("worker-completed", "another", "Checked.")
	}
	if !r.observationsReady(true) {
		t.Fatal("a full batch should be reviewed during ongoing work")
	}
}

func TestScopedQuestionsReachWaitingAndActiveWorkers(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	r := Run{ID: "team", Project: "p", State: "working", Tasks: []Task{
		{ID: "api", State: "question", Question: "Which format?"},
		{ID: "cli", State: "running"}, {ID: "docs", State: "running"},
	}}
	r.observe("worker-question", "api", r.Tasks[0].Question)
	if r.surfaceWorkerQuestions() {
		t.Fatal("prompted the human before the coordinator could answer")
	}
	source := r.Observations[0].ID
	response := map[string]any{"actions": []coordinatorAction{
		{ID: "ask", Kind: "ask", Ref: source, Question: &Question{Text: "ISO or local dates?", Tasks: []string{"cli"}, Blocking: true}},
		{ID: "notify", Kind: "message_worker", Ref: "docs", Text: "Keep the example consistent."},
	}}
	raw, _ := json.Marshal(response)
	if err := e.finishObservation(&r, Turn{Task: source}, adapter.Result{Text: string(raw)}); err != nil {
		t.Fatal(err)
	}
	if r.ObserverError != "" || !r.taskHeld("api") || !r.taskHeld("cli") || r.taskHeld("docs") || r.surfaceWorkerQuestions() {
		t.Fatalf("wrong scope for escalation: %+v, %s", r.Inbox, r.ObserverError)
	}
	// The CLI and docs workers began their turns before the question and note.
	for _, id := range []string{"cli", "docs"} {
		if err := e.workerResult(t.Context(), &r, Turn{Task: id}, adapter.Result{Text: `{"status":"committed","commit":"before-guidance"}`}); err != nil {
			t.Fatal(err)
		}
		if state := taskByID(&r, id).State; state != "pending" {
			t.Fatalf("%s accepted stale work: %s", id, state)
		}
	}
	for kind, id := range map[string]string{"answer-inbox": r.Inbox[0].ID, "answer": "api"} {
		r.ID = kind
		if err := e.db.save(&r, "waiting-for-answer"); err != nil {
			t.Fatal(err)
		}
		got, err := e.Command(t.Context(), r.ID, Command{Kind: kind, Task: id, Text: "ISO dates"})
		if err != nil || got.Tasks[0].Reply != "ISO dates" || len(got.Tasks[1].Notes) != 1 ||
			!strings.Contains(got.Tasks[1].Notes[0], "ISO dates") || len(got.Tasks[2].Notes) != 1 || got.taskHeld("cli") {
			t.Fatalf("%s missed a recipient or reached an unrelated task: %+v, %v", kind, got.Tasks, err)
		}
	}
}

func TestRecoverPreviouslyUnansweredWorkerOnce(t *testing.T) {
	r := Run{State: "working", Tasks: []Task{{ID: "old", State: "question", Question: "Which output?"}}}
	if !r.surfaceWorkerQuestions() || r.surfaceWorkerQuestions() || len(r.Inbox) != 1 || !r.taskHeld("old") {
		t.Fatalf("expected a single recoverable inbox item: %+v", r.Inbox)
	}
}
