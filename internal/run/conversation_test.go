package run

import (
	"testing"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
)

func TestConversationProposalPreservesAndExtendsAcceptance(t *testing.T) {
	e := Engine{}
	r := Run{
		Input: Start{Workers: []string{"writer"}, Recipe: eval.Recipe{Checks: [][]string{{"test-original"}}}},
		State: "review", RecipeApproved: true,
		Tasks:      []Task{{ID: "base-work", State: "committed", Worker: "writer"}, {ID: "repair", State: "committed", Worker: "writer", RepairBase: "abc"}},
		Candidates: []Candidate{{ID: "old", State: "passed"}},
	}
	if err := queueMessage(&r, "Add a check"); err != nil {
		t.Fatal(err)
	}
	id := r.Conversation[0].ID
	e.finishChat(&r, Turn{Task: id}, adapter.Result{Text: `{"reply":"Add a follow-up","tasks":[{"id":"next","goal":"Write a check","worker":"writer","class":"testing","depends":["base-work"]}],"checks":[["test-new"]]}`})
	reply := r.Conversation[1]
	if reply.ProposalState != "pending" || r.Conversation[0].State != "answered" {
		t.Fatalf("%+v", r.Conversation)
	}
	if err := e.acceptProposal(&r, reply.ID); err != nil {
		t.Fatal(err)
	}
	if r.State != "working" || r.Candidates[0].State != "superseded" || len(r.Input.Recipe.Checks) != 2 || r.Input.Recipe.Checks[0][0] != "test-original" {
		t.Fatalf("%+v", r)
	}
	if len(r.Tasks[2].Depends) != 2 || r.Tasks[2].Depends[1] != "repair" {
		t.Fatalf("%+v", r.Tasks[2])
	}
	sets, err := selections(r.Tasks)
	if err != nil || len(sets) != 1 || len(sets[0]) != 2 || sets[0][1] != "next" {
		t.Fatalf("%v %v", sets, err)
	}
	if e.acceptProposal(&r, reply.ID) == nil {
		t.Fatal("proposal applied twice")
	}
}

func TestConversationFailureDoesNotBlockRun(t *testing.T) {
	e := Engine{}
	r := Run{Input: Start{Workers: []string{"writer"}}, State: "working"}
	if err := queueMessage(&r, "Why?"); err != nil {
		t.Fatal(err)
	}
	if queueMessage(&r, "Another?") == nil {
		t.Fatal("accepted a concurrent message")
	}
	e.finishChat(&r, Turn{Task: r.Conversation[0].ID}, adapter.Result{Error: "connection failed"})
	if r.State != "working" || r.Conversation[0].State != "failed" {
		t.Fatalf("%+v", r)
	}
}
