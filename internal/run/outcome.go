package run

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/project"
)

func (e *Engine) accept(ctx context.Context, r *Run, turn Turn, result adapter.Result) error {
	if turn.Role == "chat" {
		e.finishChat(r, turn, result)
		return nil
	}
	if turn.Role == "eval" {
		return e.evalResult(r, turn, result)
	}
	if result.Error != "" {
		if t := taskByID(r, turn.Task); t != nil {
			t.Session = result.Session
			if !awaitingChildren(r, t.ID) && t.Delivered == len(t.Notes) && !r.taskHeld(t.ID) {
				if sha, err := project.Milestone(ctx, t.Workspace, t.Base); err == nil {
					return e.saveCommit(ctx, r, t, sha)
				}
			}
			t.State, t.Finding = "blocked", result.Error
			return nil
		}
		r.State, r.Detail = "blocked", result.Error
		return nil
	}
	switch turn.Role {
	case "plan":
		var plan Plan
		if err := adapter.DecodeOutcome(result.Text, &plan); err != nil {
			return err
		}
		plan.Question = strings.TrimSpace(plan.Question)
		if err := validatePlan(plan, r.Input.Workers); err != nil {
			return err
		}
		r.Session = result.Session
		if plan.Question != "" {
			r.State, r.Question = "question", plan.Question
			r.Inbox = append(r.Inbox, Question{ID: uid(), Text: plan.Question, Source: "plan", State: "open", Revision: 1, Blocking: true})
			return nil
		}
		if err := validateTasks(plan.Tasks, r.Tasks[:r.TaskStart], r.Input.Workers); err != nil {
			return err
		}
		r.Tasks = append(r.Tasks[:r.TaskStart], plan.Tasks...)
		if r.Input.Compare {
			for i := range r.activeTasks() {
				r.activeTasks()[i].Round = r.Round
			}
		}
		e.assign(r, r.activeTasks())
		r.State = "working"
		if !r.RecipeApproved {
			if err := plan.Recipe.Validate(); err != nil {
				return err
			}
			r.Proposed, r.State = plan.Recipe, "checks"
		}
	case "worker":
		return e.workerResult(ctx, r, turn, result)
	case "repair":
		var repair Repair
		if err := adapter.DecodeOutcome(result.Text, &repair); err != nil {
			return err
		}
		selected := taskByID(r, repair.Task)
		base := candidateByID(r, r.RepairID)
		if selected == nil || strings.TrimSpace(repair.Instruction) == "" || base == nil || base.Commit == "" {
			return errors.New("invalid repair assignment")
		}
		r.Session = result.Session
		added := []Task{{ID: "repair-" + uid(), Worker: selected.Worker, Goal: repair.Instruction, Class: "debugging", RepairBase: base.Commit}}
		e.assign(r, added)
		r.Tasks = append(r.Tasks, added...)
		r.State, r.RepairID, r.RevisionGoal = "working", "", ""
	}
	return nil
}

func (e *Engine) workerResult(ctx context.Context, r *Run, turn Turn, result adapter.Result) error {
	t := taskByID(r, turn.Task)
	if t == nil {
		return errors.New("worker milestone missing")
	}
	t.Session = result.Session
	var outcome WorkerResult
	if err := adapter.DecodeOutcome(result.Text, &outcome); err != nil {
		t.State, t.Finding = "blocked", err.Error()
		return nil
	}
	t.Finding = outcome.Message
	switch outcome.Status {
	case "committed":
		if awaitingChildren(r, t.ID) || t.Delivered < len(t.Notes) || r.taskHeld(t.ID) {
			// A handoff, message or blocking decision arrived after this turn started.
			t.State = "pending"
			return nil
		}
		if err := e.saveCommit(ctx, r, t, outcome.Commit); err != nil {
			t.State, t.Finding = "blocked", err.Error()
		}
	case "question":
		if strings.TrimSpace(outcome.Message) == "" {
			t.State, t.Finding = "blocked", "empty question"
			return nil
		}
		t.State, t.Question = "question", outcome.Message
		r.observe("worker-question", t.ID, outcome.Message)
	case "finding":
		recipient := taskByID(r, outcome.Recipient)
		if recipient == nil || recipient == t || outcome.Message == "" {
			t.State, t.Finding = "blocked", "invalid finding recipient"
			return nil
		}
		r.observe("worker-finding", t.ID, "For "+recipient.ID+": "+outcome.Message)
		recipient.Notes = append(recipient.Notes, t.ID+": "+outcome.Message)
		t.State = "pending"
	case "blocked":
		t.State = "blocked"
	default:
		t.State, t.Finding = "blocked", "unknown worker outcome"
	}
	return nil
}

func (e *Engine) saveCommit(ctx context.Context, r *Run, t *Task, reported string) error {
	var sha string
	var err error
	var mergeDir string
	if t.Conflict == "" {
		sha, err = project.ReportedMilestone(ctx, t.Workspace, t.Base)
	} else {
		c := candidateByID(r, t.Conflict)
		if c == nil || len(c.Pending) == 0 {
			return errors.New("repair is missing its merge inputs")
		}
		incoming := taskByID(r, c.Pending[0])
		sha, err = project.Head(ctx, t.Workspace)
		if err == nil && (incoming == nil || !project.IsAncestor(ctx, t.Workspace, incoming.Commit, sha) || !project.IsAncestor(ctx, t.Workspace, t.Base, sha)) {
			return errors.New("repair commit does not contain both merge parents")
		}
		mergeDir = filepath.Join(e.root(*r), "candidates", c.ID, "source")
	}
	if err != nil {
		return err
	}
	if sha != reported {
		return errors.New("reported commit does not match worker HEAD")
	}
	if len(t.Paths) > 0 {
		var paths []string
		if mergeDir != "" {
			paths, err = project.RepairChanges(ctx, mergeDir, t.Workspace, sha)
		} else {
			paths, err = project.ChangedPaths(ctx, t.Workspace, t.Base, sha)
		}
		if err != nil {
			return err
		}
		for _, path := range paths {
			if !allowedPath(t.Paths, path) {
				return fmt.Errorf("milestone changed %q outside its assigned scope", path)
			}
		}
	}
	ref := "refs/swarm/milestones/" + t.ID + "/" + sha
	if err = project.Import(ctx, e.integration(*r), t.Workspace, sha, ref); err != nil {
		return err
	}
	t.State, t.Commit, t.Question, t.Reply = "committed", sha, "", ""
	if t.Parent != "" {
		if parent := taskByID(r, t.Parent); parent != nil {
			parent.Notes = append(parent.Notes, "Delegated milestone "+t.ID+" committed "+sha+". "+t.Finding)
			if parent.State == "question" {
				parent.State, parent.Question = "pending", ""
			}
		}
	}
	if r.Project != "" {
		r.observe("worker-completed", t.ID, t.Finding)
	}
	if status, err := project.Status(ctx, t.Workspace); err == nil && status != "" {
		t.Finding = strings.TrimSpace(t.Finding + "\nUncommitted files remain in this worker's checkout. The candidate uses the reported commit.")
	}
	return nil
}

func selections(tasks []Task) ([][]string, error) {
	groups := map[string][]string{}
	var order, required []string
	variants := map[string][]string{}
	var variantOrder []string
	for i := len(tasks) - 1; i >= 0; i-- {
		if tasks[i].RepairBase != "" && tasks[i].Conflict == "" {
			tasks = tasks[i:]
			break
		}
	}
	children := map[string][]string{}
	for _, t := range tasks {
		if t.State == "abandoned" || t.Conflict != "" {
			continue
		}
		if t.Parent != "" {
			children[t.Parent] = append(children[t.Parent], t.ID)
			continue
		}
		if t.Variant != "" {
			if _, ok := variants[t.Variant]; !ok {
				variantOrder = append(variantOrder, t.Variant)
			}
			variants[t.Variant] = append(variants[t.Variant], t.ID)
			continue
		}
		if t.Alternative == "" {
			required = append(required, t.ID)
			continue
		}
		if _, seen := groups[t.Alternative]; !seen {
			order = append(order, t.Alternative)
		}
		groups[t.Alternative] = append(groups[t.Alternative], t.ID)
	}
	includeChildren := func(sets [][]string) [][]string {
		for i, set := range sets {
			for j := 0; j < len(set); j++ {
				set = append(set, children[set[j]]...)
			}
			sets[i] = set
		}
		return sets
	}
	if len(variantOrder) > 0 {
		if len(order) > 0 {
			return nil, errors.New("use variant memberships consistently within one experiment")
		}
		result := make([][]string, 0, len(variantOrder))
		for _, name := range variantOrder {
			result = append(result, append(slices.Clone(required), variants[name]...))
		}
		return includeChildren(result), nil
	}
	result := [][]string{required}
	for _, group := range order {
		var next [][]string
		for _, base := range result {
			for _, choice := range groups[group] {
				next = append(next, append(append([]string{}, base...), choice))
				if len(next) > 12 {
					return nil, errors.New("too many candidate combinations; use fewer alternative groups")
				}
			}
		}
		result = next
	}
	return includeChildren(result), nil
}

func (e *Engine) candidates(ctx context.Context, r *Run) error {
	sets, err := selections(r.activeTasks())
	if err != nil {
		return err
	}
	base := r.workBase()
	if r.Repo.Target != "" {
		base, err = project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
		if err != nil {
			return err
		}
		if err = project.Fetch(ctx, e.integration(*r), r.Repo.Root, base); err != nil {
			return err
		}
	}
	queue := func() {
		if r.Input.Compare {
			r.Candidates = append(r.Candidates, Candidate{ID: uid(), Round: r.Round, Policy: r.Policy, Base: base, State: "queued", Baseline: true, Name: "Baseline"})
		}
		for _, set := range sets {
			c := Candidate{ID: uid(), Base: base, Tasks: set, State: "queued", Round: r.Round, Policy: r.Policy}
			var names []string
			for _, id := range set {
				t := taskByID(r, id)
				if t.Variant != "" {
					names = append(names, t.Variant)
				} else if t.Alternative != "" || t.RepairBase != "" {
					names = append(names, t.Goal)
				}
				c.Parents = append(c.Parents, t.Donors...)
			}
			if len(names) == 0 {
				names = set
			}
			c.Name = strings.Join(slices.Compact(names), " · ")
			r.Candidates = append(r.Candidates, c)
		}
	}
	queue()
	r.State = "evaluating"
	return e.db.save(r, "candidates-queued")
}

func (e *Engine) candidateStep(ctx context.Context, r *Run) error {
	for i := range r.Candidates {
		c := &r.Candidates[i]
		if c.Round != r.Round || c.Policy != r.Policy {
			continue
		}
		if c.State == "evaluating" {
			return nil
		}
		if c.State == "conflict" {
			before := r.Revision
			if err := e.advanceConflict(ctx, r, c); err != nil || r.Revision != before {
				return err
			}
			continue
		}
		if c.State != "queued" {
			continue
		}
		if len(r.Input.Recipe.Cases) > 0 {
			busy, err := e.measurementBusy(r.ID)
			if err != nil || busy {
				return err
			}
		}
		dir := filepath.Join(e.root(*r), "candidates", c.ID, "source")
		if _, err := os.Stat(dir); err == nil {
			// No turn intent exists for a queued candidate. Discard only its
			// interrupted staging directory and rebuild from the pinned inputs.
			if err := os.RemoveAll(dir); err != nil {
				return err
			}
		}
		if err := project.Clone(ctx, e.integration(*r), dir, c.Base, "swarm-candidate"); err != nil {
			return err
		}
		if !c.Baseline {
			if err := project.ApplyCommitPolicy(ctx, r.Repo.Root, dir); err != nil {
				return err
			}
		}
		for index, id := range c.Tasks {
			t := taskByID(r, id)
			if t == nil {
				return errors.New("candidate milestone missing")
			}
			sha := t.Commit
			if err := project.Fetch(ctx, dir, e.integration(*r), sha); err != nil {
				return err
			}
			if err := project.Merge(ctx, dir, sha); err != nil {
				return e.repairConflict(ctx, r, c, dir, c.Tasks[index:])
			}
		}
		sha, err := project.Head(ctx, dir)
		if err != nil {
			return err
		}
		if err := project.Import(ctx, e.integration(*r), dir, sha, "refs/swarm/candidates/"+c.ID); err != nil {
			return err
		}
		c.Commit = sha
		c.Preview = slices.Clone(r.Input.Recipe.Preview)
		return e.dispatchEvaluation(r, c, dir)
	}
	passed := false
	for _, c := range r.Candidates {
		if c.Round != r.Round || c.Policy != r.Policy {
			continue
		}
		if c.State == "conflict" {
			return nil
		}
		if c.State == "passed" && !c.Baseline {
			passed = true
		}
	}
	if passed || r.Input.Compare || r.Input.Recipe.Rule != nil || r.Input.Recipe.Rubric != "" {
		for i := range r.Candidates {
			c := &r.Candidates[i]
			if c.Round == r.Round && c.Policy == r.Policy {
				ok := eligible(r.Input.Recipe.Rule, *c)
				c.Eligible = &ok
			}
		}
		s := recommend(r)
		r.Selection = &s
		r.State, r.Detail = "review", ""
		if r.Input.Compare || r.Input.Delivery != "" && r.Input.Delivery != "review" || len(r.Input.Recipe.Cases) > 0 {
			r.observe("comparison-ready", "", s.Reason)
		}
		return e.db.save(r, "review")
	}
	for i := len(r.Candidates) - 1; i >= 0; i-- {
		last := r.Candidates[i]
		if last.Round != r.Round || last.Policy != r.Policy || last.Baseline || last.Commit == "" || last.State != "failed" {
			continue
		}
		if !slices.ContainsFunc(r.Tasks, func(t Task) bool { return t.RepairBase != "" }) {
			r.State, r.RepairID = "repairing", last.ID
			return e.db.save(r, "repair-needed")
		}
		break
	}
	r.State, r.Detail = "blocked", "No candidate passed. Inspect the eval evidence."
	return e.db.save(r, "checks-failed")
}

func (e *Engine) evalResult(r *Run, turn Turn, result adapter.Result) error {
	c := candidateByID(r, turn.Task)
	if c == nil {
		return errors.New("eval candidate missing")
	}
	c.State, c.Detail, c.Log = "passed", "", filepath.Join(e.jobdir(*r, turn.ID), "events.log")
	if c.Inputs != "" {
		if err := e.recordReport(r, c, turn.ID); err != nil {
			c.State, c.Detail = "inconclusive", err.Error()
		}
	}
	if result.Error != "" && c.State != "inconclusive" {
		c.State, c.Detail = "failed", result.Error
	}
	if r.Input.Compare || result.Error != "" || c.Inputs != "" {
		r.observe("eval-"+c.State, c.ID, c.Detail)
	}
	return nil
}

func (e *Engine) repair(r *Run) error {
	c := candidateByID(r, r.RepairID)
	if c == nil {
		return errors.New("candidate for repair is missing")
	}
	log, _ := os.ReadFile(c.Log)
	if len(log) > 12000 {
		log = log[len(log)-12000:]
	}
	tasks, _ := json.Marshal(r.Tasks)
	prompt := fmt.Sprintf(`Revise the integrated candidate %s. Select the worker most likely to address the request or failed eval using milestone IDs and evidence. Return only {"task":"milestone-id","instruction":"specific implementation goal"}. The revision must be a normal commit on this candidate.
Brief: %s
Developer request: %s
Milestones: %s
Eval error: %s
Eval log:
%s`, c.Commit, r.Input.Prompt, r.RevisionGoal, tasks, c.Detail, log)
	return e.dispatch(r, "repair", "", job{Request: adapter.Request{Profile: r.Profiles[r.Input.Coordinator], Role: "coordinator", Dir: e.integration(*r), Prompt: prompt, Session: r.Session}})
}

var ErrStaleRevision = errors.New("the run changed; review its current state and retry")

type Command struct {
	ID       string   `json:"id,omitempty"`
	Expected *int64   `json:"expected_revision,omitempty"`
	Donors   []string `json:"donors,omitempty"`

	Kind      string       `json:"kind"`
	Task      string       `json:"task,omitempty"`
	Text      string       `json:"text,omitempty"`
	Candidate string       `json:"candidate,omitempty"`
	Tokens    int64        `json:"tokens,omitempty"`
	Recipe    *eval.Recipe `json:"recipe,omitempty"`
}

func (e *Engine) Command(ctx context.Context, id string, action Command) (Run, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	r, err := e.db.get(id)
	if err != nil {
		return r, err
	}
	var commandDigest string
	if action.ID != "" {
		body, err := json.Marshal(action)
		if err != nil {
			return r, err
		}
		commandDigest = fmt.Sprintf("%x", sha256.Sum256(body))
		if !idPattern.MatchString(action.ID) {
			return r, errors.New("invalid command ID")
		}
		if r.Commands == nil {
			r.Commands = map[string]string{}
		}
		if previous, ok := r.Commands[action.ID]; ok {
			if previous != commandDigest && previous != action.Kind {
				return r, errors.New("command ID already used")
			}
			return r, nil
		}
	}
	if action.Expected != nil && r.Revision != *action.Expected {
		return r, ErrStaleRevision
	}
	if action.ID != "" {
		r.Commands[action.ID] = commandDigest
	}
	switch action.Kind {
	case "answer", "answer-inbox", "select", "keep-baseline", "combine", "new-round", "feedback", "vote", "change", "approve", "update-suite", "land":
		r.Interventions++
	}
	handled, err := e.experimentCommand(ctx, &r, action)
	if err != nil {
		return r, err
	}
	if handled {
		return r, e.db.save(&r, action.Kind)
	}
	if r.State == "landed" && action.Kind != "land" && action.Kind != "chat" && action.Kind != "dismiss-proposal" {
		return r, errors.New("run is already landed")
	}
	switch action.Kind {
	case "chat":
		if err := queueMessage(&r, action.Text); err != nil {
			return r, err
		}
	case "accept-proposal":
		if err := e.acceptProposal(&r, action.Task); err != nil {
			return r, err
		}
	case "dismiss-proposal":
		msg := messageByID(&r, action.Task)
		if msg == nil || msg.ProposalState != "pending" {
			return r, errors.New("proposal is no longer open")
		}
		msg.ProposalState = "dismissed"
	case "approve":
		if r.State != "checks" {
			return r, errors.New("no recipe waiting for approval")
		}
		recipe := r.Proposed
		if action.Recipe != nil {
			recipe = *action.Recipe
		}
		if err := recipe.Validate(); err != nil {
			return r, err
		}
		pinned, err := e.pinRecipe(&r, recipe)
		if err != nil {
			return r, err
		}
		r.Input.Recipe, r.RecipeApproved, r.State = pinned, true, "working"
		r.notePolicy()
	case "answer":
		if strings.TrimSpace(action.Text) == "" {
			return r, errors.New("answer is required")
		}
		if action.Task == "" && r.State == "question" {
			r.State, r.Question, r.Answer = "planning", "", action.Text
			for i := range r.Inbox {
				if r.Inbox[i].State == "open" && r.Inbox[i].Source == "plan" {
					r.Inbox[i].State, r.Inbox[i].Answer = "answered", action.Text
				}
			}
		} else if t := taskByID(&r, action.Task); t != nil && t.State == "question" {
			for i := range r.Inbox {
				if r.Inbox[i].State == "open" && slices.Contains(r.Inbox[i].Tasks, t.ID) {
					r.recordInboxAnswer(&r.Inbox[i], action.Text)
				}
			}
			t.State, t.Question, t.Reply = "pending", "", action.Text
		} else {
			return r, errors.New("question is no longer open")
		}
	case "coordinator":
		if action.Text == r.Input.Coordinator {
			return r, nil
		}
		if _, ok := r.Profiles[action.Text]; !ok {
			return r, errors.New("profile unavailable")
		}
		r.Generation++
		r.ChatSession, r.ObserverSession, r.ObserverError = "", "", ""
		for i := range r.Conversation {
			if r.Conversation[i].State == "running" {
				r.Conversation[i].State = "queued"
			}
		}
		r.Input.Coordinator, r.Session = action.Text, ""
		if r.State == "blocked" && len(r.Tasks) == 0 {
			r.State, r.Detail = "planning", ""
		}
	case "stop":
		if r.State != "stopped" {
			r.StoppedFrom = r.State
		}
		r.State = "stopped"
		if err := e.db.save(&r, "stop-requested"); err != nil {
			return r, err
		}
		for _, turn := range r.Turns {
			if turn.State == "running" {
				if err := stopJob(e.jobdir(r, turn.ID)); err != nil {
					return r, err
				}
			}
		}
	case "resume":
		if r.State != "stopped" && r.State != "blocked" && r.State != "budget" && r.State != "working" {
			return r, errors.New("run is already active or awaiting another action")
		}
		for _, turn := range r.Turns {
			if turn.State == "running" && r.State == "stopped" {
				return r, errors.New("previous turns are still stopping")
			}
		}
		r.ObserverError = ""
		if action.Tokens < 0 {
			return r, errors.New("token target cannot be negative")
		}
		r.Input.TokenTarget = action.Tokens
		r.State = resumePhase(&r)
		r.StoppedFrom = ""
		if r.State == "evaluating" {
			for i := range r.Candidates {
				c := &r.Candidates[i]
				if c.Round == r.Round && c.Policy == r.Policy && c.State == "cancelled" {
					next := Candidate{ID: uid(), Base: c.Base, Tasks: c.Tasks, State: "queued", Round: c.Round, Policy: c.Policy,
						Name: c.Name, Baseline: c.Baseline, Parents: c.Parents}
					c.State, c.Detail = "superseded", "Verification stopped. Retrying as "+next.ID
					r.Candidates = append(r.Candidates, next)
				}
			}
		}
		for i := r.TaskStart; i < len(r.Tasks); i++ {
			t := &r.Tasks[i]
			if (action.Task == "" || action.Task == t.ID) && (t.State == "blocked" || t.State == "interrupted") {
				t.State = "pending"
			}
		}
	case "change":
		if r.State != "review" || strings.TrimSpace(action.Text) == "" {
			return r, errors.New("select a reviewed candidate and describe the change")
		}
		c := candidateByID(&r, action.Candidate)
		if c == nil || c.State != "passed" {
			return r, errors.New("select a passing candidate")
		}
		for i := range r.Candidates {
			if r.Candidates[i].State == "passed" {
				r.Candidates[i].State = "superseded"
			}
		}
		r.State, r.RepairID, r.RevisionGoal = "repairing", c.ID, action.Text
	case "land":
		if r.Repo.Target == "" && r.State == "review" && action.Text != "" {
			target := "refs/heads/" + strings.TrimPrefix(action.Text, "refs/heads/")
			if _, err := project.TargetHead(ctx, r.Repo.Root, target); err != nil {
				return r, err
			}
			r.Repo.Target = target
		}
		return e.land(ctx, r, action.Candidate)
	default:
		return r, errors.New("unknown action")
	}
	return r, e.db.save(&r, action.Kind)
}

func resumePhase(r *Run) string {
	if r.State == "stopped" {
		switch r.StoppedFrom {
		case "planning", "question", "checks", "working", "evaluating", "review", "repairing":
			return r.StoppedFrom
		case "":
			// Recover older stopped records from their outstanding work.
			switch {
			case r.Question != "":
				return "question"
			case r.RepairID != "":
				return "repairing"
			case slices.ContainsFunc(r.Candidates, func(c Candidate) bool {
				return c.Round == r.Round && c.Policy == r.Policy && slices.Contains([]string{"queued", "conflict", "cancelled"}, c.State)
			}):
				return "evaluating"
			case r.Selection != nil:
				return "review"
			}
		}
	}
	if len(r.activeTasks()) == 0 {
		return "planning"
	}
	if !r.RecipeApproved {
		return "checks"
	}
	return "working"
}

func (e *Engine) land(ctx context.Context, r Run, id string) (Run, error) {
	c := candidateByID(&r, id)
	if r.State == "landed" && c != nil && r.Detail == c.Commit {
		return r, nil
	}
	if r.ObserverError != "" && r.Input.Delivery != "review" && r.Input.Delivery != "" {
		return r, errors.New("retry the coordinator or switch to manual review")
	}
	if r.State != "review" || c == nil || !eligible(r.Input.Recipe.Rule, *c) || c.Round != r.Round || c.Policy != r.Policy ||
		r.Repo.Target == "" || r.waitingForSelection() || slices.ContainsFunc(c.Tasks, r.taskHeld) || r.Combination != nil {
		return r, errors.New("select a passing candidate and a local target branch")
	}
	actual, err := project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
	if err != nil {
		return r, err
	}
	if actual != c.Base && actual != c.Commit {
		for i := range r.Candidates {
			if r.Candidates[i].Base != actual && r.Candidates[i].State == "passed" {
				r.Candidates[i].State, r.Candidates[i].Detail = "stale", "the target moved; rebuilding against its current head"
			}
		}
		if err := e.candidates(ctx, &r); err != nil {
			return r, err
		}
		return r, nil
	}
	if r.Selection == nil || r.Selection.Candidate != c.ID {
		s := recommend(&r)
		s.Candidate, s.State, s.Override, s.Reason = c.ID, "selected", true, "You selected this implementation."
		r.Selection = &s
	}
	r.Landing = c.ID
	if err = e.db.save(&r, "landing"); err != nil {
		return r, err
	}
	if err = project.Import(ctx, r.Repo.Root, e.integration(r), c.Commit, "refs/swarm/runs/"+r.ID+"/"+c.ID); err != nil {
		return r, err
	}
	if err = project.Land(ctx, r.Repo.Root, r.Repo.Target, c.Base, c.Commit); err != nil {
		head, inspectErr := project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
		if inspectErr == nil && head == c.Base {
			r.Landing = ""
			r.Held = true
			r.Detail = err.Error()
			if saveErr := e.db.save(&r, "landing-wait"); saveErr != nil {
				return r, saveErr
			}
		}
		return r, err
	}
	e.markLanded(&r, c)
	return r, e.db.save(&r, "landed")
}

func (e *Engine) markLanded(r *Run, c *Candidate) {
	r.State, r.Detail, r.Landing = "landed", c.Commit, ""
	now := time.Now().UTC()
	if r.Selection == nil || r.Selection.Candidate != c.ID {
		r.Selection = &Selection{Candidate: c.ID, Round: r.Round, Policy: r.Policy, At: now}
	}
	r.Selection.State, r.Selection.LandedAt = "landed", &now
	if r.Input.Delivery == "deploy" {
		r.Releases = append(r.Releases, Release{ID: uid(), Candidate: c.ID, Commit: c.Commit, Destination: r.Input.Destination, State: "queued"})
	}
}
