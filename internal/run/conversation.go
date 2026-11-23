package run

import (
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"slices"
	"strings"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
)

type Message struct {
	ID            string     `json:"id"`
	Role          string     `json:"role"`
	Text          string     `json:"text"`
	State         string     `json:"state,omitempty"`
	Error         string     `json:"error,omitempty"`
	Tasks         []Task     `json:"tasks,omitempty"`
	Checks        [][]string `json:"checks,omitempty"`
	ProposalState string     `json:"proposal_state,omitempty"`
}

type chatResult struct {
	Reply  string     `json:"reply"`
	Tasks  []Task     `json:"tasks,omitempty"`
	Checks [][]string `json:"checks,omitempty"`
}

func messageByID(r *Run, id string) *Message {
	for i := range r.Conversation {
		if r.Conversation[i].ID == id {
			return &r.Conversation[i]
		}
	}
	return nil
}

func pendingMessage(r *Run) *Message {
	for i := range r.Conversation {
		if r.Conversation[i].State == "queued" {
			return &r.Conversation[i]
		}
	}
	return nil
}

func queueMessage(r *Run, text string) error {
	text = strings.TrimSpace(text)
	if text == "" || len(text) > 6000 {
		return errors.New("write a message of 1 to 6,000 bytes")
	}
	if r.State == "stopped" {
		return errors.New("resume the run before sending a message")
	}
	if r.Input.TokenTarget > 0 && r.Tokens >= r.Input.TokenTarget {
		return errors.New("the run reached its token target")
	}
	if len(r.Conversation) >= 200 {
		return errors.New("this run's conversation is full")
	}
	for _, msg := range r.Conversation {
		if msg.State == "queued" || msg.State == "running" {
			return errors.New("the coordinator is still answering")
		}
	}
	r.Conversation = append(r.Conversation, Message{ID: uid(), Role: "user", Text: text, State: "queued"})
	return nil
}

func (e *Engine) dispatchChat(r *Run, msg *Message) error {
	type milestone struct {
		ID, Goal, Worker, State, Finding, Commit string
		Depends                                  []string
		Route                                    any
	}
	tasks := make([]milestone, 0, len(r.Tasks))
	for _, task := range r.Tasks {
		tasks = append(tasks, milestone{task.ID, task.Goal, task.Worker, task.State, task.Finding, task.Commit, task.Depends, task.Route})
	}
	summary, err := json.Marshal(tasks)
	if err != nil {
		return err
	}
	type candidateSummary struct{ Commit, State, Detail string }
	candidates := make([]candidateSummary, 0, len(r.Candidates))
	for _, c := range r.Candidates {
		candidates = append(candidates, candidateSummary{c.Commit, c.State, c.Detail})
	}
	candidateText, err := json.Marshal(candidates)
	if err != nil {
		return err
	}
	seen := r.Conversation[max(0, len(r.Conversation)-12):]
	var history strings.Builder
	for _, message := range seen {
		if message.ID != msg.ID {
			fmt.Fprintf(&history, "\n%s: %s", message.Role, message.Text)
		}
	}
	prompt := fmt.Sprintf(`You are the Pantheon Local coordinator answering the developer. Explain the run and provide reasoning from the milestone and candidate state. Treat prior messages and run evidence as context. Use no commands or tools. Only if the developer asks to add work, propose new milestones with distinct IDs. Existing milestones stay as they are. Each proposed task must have goal, a fallback worker from %q, a class (implementation, debugging, testing, automation, documentation, or design), and optionally depends and paths. Propose any additional argv acceptance checks for new behavior. The developer decides whether to add the proposal. Return only JSON: {"reply":"answer","tasks":[],"checks":[]}.
Run state: %s
Original brief: %s
Milestones: %s
Candidates: %s
Recent conversation: %s
Developer message: %s`, r.Input.Workers, r.State, r.Input.Prompt, summary, candidateText, history.String(), msg.Text)
	msg.State = "running"
	return e.dispatch(r, "chat", msg.ID, job{Request: adapter.Request{Profile: r.Profiles[r.Input.Coordinator], Role: "chat",
		Dir: filepath.Join(e.root(*r), "coordinator"), Prompt: prompt, Session: r.ChatSession}})
}

func (e *Engine) finishChat(r *Run, turn Turn, result adapter.Result) {
	msg := messageByID(r, turn.Task)
	if msg == nil {
		return
	}
	if result.Error != "" {
		msg.State, msg.Error = "failed", result.Error
		return
	}
	var outcome chatResult
	err := adapter.DecodeOutcome(result.Text, &outcome)
	if err != nil || strings.TrimSpace(outcome.Reply) == "" || len(outcome.Reply) > 12000 {
		msg.State, msg.Error = "failed", "The coordinator did not return a usable reply."
		return
	}
	msg.State = "answered"
	r.ChatSession = result.Session
	reply := Message{ID: uid(), Role: "coordinator", Text: outcome.Reply}
	if len(outcome.Tasks) > 0 {
		err = validateTasks(outcome.Tasks, r.Tasks, r.Input.Workers)
		if err == nil && len(outcome.Checks) > 0 {
			err = (eval.Recipe{Checks: outcome.Checks}).Validate()
		}
		if err == nil {
			reply.Tasks, reply.Checks, reply.ProposalState = outcome.Tasks, outcome.Checks, "pending"
		} else {
			reply.Error = "The proposed milestones could not be added: " + err.Error()
		}
	}
	r.Conversation = append(r.Conversation, reply)
}

func canAddMilestones(state string) bool {
	return state == "working" || state == "review"
}

func (e *Engine) acceptProposal(r *Run, id string) error {
	msg := messageByID(r, id)
	if msg == nil || msg.ProposalState != "pending" {
		return errors.New("proposal is no longer open")
	}
	if !canAddMilestones(r.State) || r.Landing != "" {
		return errors.New("add milestones when the run is working or ready for review")
	}
	tasks := slices.Clone(msg.Tasks)
	for i := len(r.Tasks) - 1; i >= 0; i-- {
		if r.Tasks[i].RepairBase == "" {
			continue
		}
		for j := range tasks {
			if !slices.Contains(tasks[j].Depends, r.Tasks[i].ID) {
				tasks[j].Depends = append(slices.Clone(tasks[j].Depends), r.Tasks[i].ID)
			}
		}
		break
	}
	if err := validateTasks(tasks, r.Tasks, r.Input.Workers); err != nil {
		return err
	}
	e.assign(r, tasks)
	r.Tasks = append(r.Tasks, tasks...)
	r.Input.Recipe.Checks = append(r.Input.Recipe.Checks, msg.Checks...)
	if len(msg.Checks) > 0 {
		r.Policy++
		r.notePolicy()
	}
	e.archiveSelection(r)
	for i := range r.Candidates {
		if r.Candidates[i].State == "passed" {
			r.Candidates[i].State = "superseded"
		}
	}
	if r.State == "review" {
		r.State = "working"
	}
	msg.ProposalState = "added"
	return nil
}
