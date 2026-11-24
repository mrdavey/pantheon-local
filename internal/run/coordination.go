package run

import (
	"encoding/json"
	"slices"
	"strings"

	"github.com/mrdavey/pantheon-local/internal/eval"
)

// Build context from the saved run so a new native session gets the same
// decisions. The recipe's case inputs, expected answers and drivers stay private.
func (r *Run) turnContext(worker *Task) ([]byte, error) {
	type decision struct {
		Question string `json:"question,omitempty"`
		Answer   string `json:"answer"`
	}
	type peer struct {
		ID    string   `json:"id"`
		Goal  string   `json:"goal"`
		Paths []string `json:"paths,omitempty"`
	}
	context := struct {
		Decisions []decision `json:"planning_decisions,omitempty"`
		RoundGoal string     `json:"round_goal,omitempty"`
		Approved  bool       `json:"checks_approved"`
		Checks    [][]string `json:"checks,omitempty"`
		Rule      *eval.Rule `json:"selection_rule,omitempty"`
		Rubric    string     `json:"review_criteria,omitempty"`
		Peers     []peer     `json:"peer_milestones,omitempty"`
	}{RoundGoal: r.RoundGoal, Approved: r.RecipeApproved}
	for _, q := range r.Inbox {
		if q.Source == "plan" && q.State == "answered" {
			context.Decisions = append(context.Decisions, decision{q.Text, q.Answer})
		}
	}
	if len(context.Decisions) == 0 && r.Answer != "" {
		context.Decisions = append(context.Decisions, decision{Answer: r.Answer})
	}
	if r.RecipeApproved {
		context.Checks, context.Rule, context.Rubric = r.Input.Recipe.Checks, r.Input.Recipe.Rule, r.Input.Recipe.Rubric
	}
	if worker != nil {
		for _, t := range r.activeTasks() {
			if t.ID != worker.ID && t.Variant == worker.Variant && t.Alternative == "" &&
				slices.Contains([]string{"pending", "running", "question"}, t.State) {
				context.Peers = append(context.Peers, peer{t.ID, t.Goal, t.Paths})
			}
		}
	}
	return json.Marshal(context)
}

// A coordinator may consume a worker's observation without answering it.
// Surface that question once, including for runs saved by an earlier version.
func (r *Run) surfaceWorkerQuestions() bool {
	changed := false
	for _, t := range r.activeTasks() {
		if t.State != "question" || strings.TrimSpace(t.Question) == "" || awaitingChildren(r, t.ID) {
			continue
		}
		if slices.ContainsFunc(r.Observations, func(o Observation) bool { return o.Kind == "worker-question" && o.Ref == t.ID }) ||
			slices.ContainsFunc(r.Inbox, func(q Question) bool { return q.State == "open" && slices.Contains(q.Tasks, t.ID) }) {
			continue
		}
		r.Inbox = append(r.Inbox, Question{ID: uid(), Text: t.Question, Source: "worker", Tasks: []string{t.ID}, Blocking: true, Revision: 1, State: "open"})
		changed = true
	}
	return changed
}

func (r *Run) recordInboxAnswer(q *Question, answer string) {
	q.State, q.Answer = "answered", answer
	for _, id := range q.Tasks {
		t := taskByID(r, id)
		if t == nil {
			continue
		}
		switch t.State {
		case "question":
			t.State, t.Question, t.Reply = "pending", "", answer
		case "running", "pending", "interrupted", "blocked":
			t.Notes = append(t.Notes, "Developer answered "+q.Text+" — "+answer)
		}
	}
}
