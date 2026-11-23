package run

import (
	"testing"

	"github.com/mrdavey/pantheon-local/internal/eval"
)

func TestStopResumePreservesDecisionAndWork(t *testing.T) {
	for _, phase := range []string{"question", "checks", "review", "repairing", "evaluating", "working"} {
		t.Run(phase, func(t *testing.T) {
			e, err := Open(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			defer e.Close()
			r := Run{ID: "r", State: phase, RecipeApproved: true, Tasks: []Task{{ID: "done", State: "committed"}}}
			switch phase {
			case "question":
				r.Tasks = nil
				r.Question = "Which format?"
			case "checks":
				r.RecipeApproved = false
			case "repairing":
				r.RepairID = "chosen"
			case "review":
				r.Selection = &Selection{Candidate: "chosen", State: "selected"}
			case "evaluating":
				r.Candidates = []Candidate{{ID: "complete", State: "passed"}, {ID: "stopped", State: "cancelled", Base: "original", Tasks: []string{"done"}}}
			}
			if err := e.db.save(&r, "seed"); err != nil {
				t.Fatal(err)
			}
			for range 2 {
				if _, err := e.Command(t.Context(), r.ID, Command{Kind: "stop"}); err != nil {
					t.Fatal(err)
				}
			}
			got, err := e.Command(t.Context(), r.ID, Command{Kind: "resume"})
			if err != nil || got.State != phase {
				t.Fatalf("resumed %s as %s: %v", phase, got.State, err)
			}
			if phase == "evaluating" && (len(got.Candidates) != 3 || got.Candidates[0].ID != "complete" || got.Candidates[0].State != "passed" || got.Candidates[2].Base != "original" || got.Candidates[2].State != "queued" || got.Candidates[2].ID == "stopped") {
				t.Fatalf("retry candidates: %+v", got.Candidates)
			}
		})
	}
}

func TestAutoLandWaitsForDeveloperConversation(t *testing.T) {
	for _, message := range []Message{{State: "queued"}, {State: "running"}, {ProposalState: "pending"}, {State: "answered"}} {
		e, err := Open(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		r := Run{ID: "r", State: "review", Input: Start{Delivery: "auto", Recipe: eval.Recipe{Checks: [][]string{{"true"}}}},
			Experiment: Experiment{Selection: &Selection{State: "recommended", Recommended: "a"}}, Conversation: []Message{message}}
		if err := e.maybeLand(t.Context(), &r); err != nil {
			t.Fatal(err)
		}
		want := "recommended"
		if message.State == "answered" {
			want = "selected"
		}
		if r.Selection.State != want {
			t.Errorf("message %+v: selection %s, want %s", message, r.Selection.State, want)
		}
		e.Close()
	}
}

func TestRoundChangeReleasesConversation(t *testing.T) {
	e := Engine{}
	r := Run{State: "review", Conversation: []Message{{ID: "old", State: "running"}, {Role: "coordinator", ProposalState: "pending"}}}
	if err := e.newRound(t.Context(), &r, "Try another approach"); err != nil {
		t.Fatal(err)
	}
	if r.Conversation[0].State != "failed" || r.Conversation[1].ProposalState == "pending" {
		t.Fatalf("old work still active: %+v", r.Conversation)
	}
	if err := queueMessage(&r, "Explain this round"); err != nil {
		t.Fatal(err)
	}
}

func TestAnswerPausedPlanInInbox(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	r := Run{ID: "run", State: "question", Question: "Which format?", Experiment: Experiment{Inbox: []Question{{ID: "q", Source: "plan", State: "open"}}}}
	if err := e.db.save(&r, "question"); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Command(t.Context(), r.ID, Command{Kind: "stop"}); err != nil {
		t.Fatal(err)
	}
	got, err := e.Command(t.Context(), r.ID, Command{Kind: "answer-inbox", Task: "q", Text: "CSV"})
	if err != nil || got.State != "stopped" || got.Inbox[0].State != "answered" {
		t.Fatalf("paused answer: %+v %v", got, err)
	}
	got, err = e.Command(t.Context(), r.ID, Command{Kind: "resume"})
	if err != nil || got.State != "planning" || got.Answer != "CSV" || got.Question != "" {
		t.Fatalf("lost answer: %+v %v", got, err)
	}
}
