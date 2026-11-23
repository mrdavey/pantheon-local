package run

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/project"
	"github.com/mrdavey/pantheon-local/internal/routing"
)

type Update struct {
	Number int64  `json:"number"`
	Kind   string `json:"kind"`
	Run    Run    `json:"run"`
}

type Engine struct {
	mu     sync.Mutex
	dir    string
	db     *store
	launch func(string) error
}

func Open(dir string) (*Engine, error) {
	db, err := openStore(dir)
	if err != nil {
		return nil, err
	}
	return &Engine{dir: dir, db: db, launch: spawn}, nil
}
func (e *Engine) Close() error                         { return e.db.db.Close() }
func (e *Engine) List() ([]Run, error)                 { return e.db.list() }
func (e *Engine) Get(id string) (Run, error)           { return e.db.get(id) }
func (e *Engine) Events(after int64) ([]Update, error) { return e.db.events(after) }
func (e *Engine) root(r Run) string                    { return filepath.Join(e.dir, "runs", r.ID) }
func (e *Engine) integration(r Run) string             { return filepath.Join(e.root(r), "integration") }
func (e *Engine) jobdir(r Run, id string) string       { return filepath.Join(e.root(r), "turns", id) }
func uid() string                                      { var b [12]byte; _, _ = rand.Read(b[:]); return hex.EncodeToString(b[:]) }

func (e *Engine) Start(ctx context.Context, input Start, profiles map[string]adapter.Profile) (Run, error) {
	if !routing.ModeValid(input.Routing) {
		return Run{}, errors.New("unknown benchmark routing mode")
	}
	if strings.TrimSpace(input.Prompt) == "" || input.Path == "" || len(input.Workers) == 0 {
		return Run{}, errors.New("choose a project, brief, coordinator, and worker")
	}
	if input.TokenTarget < 0 {
		return Run{}, errors.New("token target cannot be negative")
	}
	if len(input.Recipe.Checks) > 0 || len(input.Recipe.Cases) > 0 || input.Recipe.Rubric != "" {
		if err := input.Recipe.Validate(); err != nil {
			return Run{}, err
		}
	}
	if err := e.validateDelivery(input); err != nil {
		return Run{}, err
	}
	for _, name := range append([]string{input.Coordinator}, input.Workers...) {
		p, ok := profiles[name]
		if !ok {
			return Run{}, fmt.Errorf("profile %q is unavailable", name)
		}
		if err := p.Validate(); err != nil {
			return Run{}, err
		}
		if _, err := exec.LookPath(p.Binary()); err != nil {
			return Run{}, fmt.Errorf("profile %q: %w", name, err)
		}
	}
	var catalog routing.Catalog
	if input.Routing != "" && input.Routing != "manual" {
		catalog = routing.Read(ctx, filepath.Join(e.dir, "benchmark-cache.json"), filepath.Join(e.dir, "benchmarks.json"), time.Now().UTC())
		if err := ctx.Err(); err != nil {
			return Run{}, err
		}
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	repo, err := project.Open(ctx, input.Path, input.InitialFiles, !input.SkipInit)
	if err != nil {
		return Run{}, err
	}
	if input.Target != "" {
		target := "refs/heads/" + strings.TrimPrefix(input.Target, "refs/heads/")
		head, err := project.TargetHead(ctx, repo.Root, target)
		if err != nil {
			return Run{}, err
		}
		if !project.IsAncestor(ctx, repo.Root, head, repo.Head) {
			return Run{}, errors.New("target must be an ancestor of the chosen base")
		}
		repo.Target = target
	}
	r := Run{ID: uid(), Input: input, Profiles: maps.Clone(profiles), Repo: repo, State: "planning", Generation: 1, RecipeApproved: len(input.Recipe.Checks) > 0 || len(input.Recipe.Cases) > 0 || input.Recipe.Rubric != ""}
	r.Project, err = e.db.projectID(repo.Common)
	if err != nil {
		return Run{}, err
	}
	r.Round, r.Policy = 1, 1
	now := time.Now().UTC()
	r.Started, r.RoundStarted = &now, &now
	if err := e.authorizeDestination(&r); err != nil {
		return Run{}, err
	}
	if r.RecipeApproved {
		r.Input.Recipe, err = e.pinRecipe(&r, input.Recipe)
		if err != nil {
			return Run{}, err
		}
		r.notePolicy()
	}
	if err = project.Clone(ctx, repo.Root, e.integration(r), repo.Head, "swarm-integration"); err != nil {
		return Run{}, err
	}
	if err = project.ApplyCommitPolicy(ctx, repo.Root, e.integration(r)); err != nil {
		return Run{}, err
	}
	if err = os.MkdirAll(filepath.Join(e.root(r), "coordinator"), 0700); err != nil {
		return Run{}, err
	}
	if input.Routing != "" && input.Routing != "manual" {
		r.BenchmarkNote = catalog.Note
		if !catalog.Fetched.IsZero() {
			r.BenchmarkFetched = catalog.Fetched.Format(time.RFC3339)
		}
		if err := writeJSON(filepath.Join(e.root(r), "benchmarks.json"), catalog); err != nil {
			return Run{}, err
		}
	}
	if err = e.db.save(&r, "started"); err != nil {
		return Run{}, err
	}
	return r, nil
}

func (e *Engine) dispatch(r *Run, role, task string, spec job) error {
	id := uid()
	dir := e.jobdir(*r, id)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(dir, "job.json"), spec); err != nil {
		return err
	}
	turn := Turn{ID: id, Task: task, Role: role, Generation: r.Generation, State: "running", Round: r.Round, Policy: r.Policy}
	if role == "worker" {
		if t := taskByID(r, task); t != nil {
			turn.Messages = len(t.Notes)
		}
	}
	r.Turns = append(r.Turns, turn)
	if err := e.db.save(r, "dispatched"); err != nil {
		return err
	}
	if err := e.launch(dir); err != nil {
		return writeJSON(filepath.Join(dir, "result.json"), adapter.Result{Error: err.Error()})
	}
	return nil
}

func (e *Engine) Loop(ctx context.Context, log io.Writer) {
	ticker := time.NewTicker(300 * time.Millisecond)
	defer ticker.Stop()
	for {
		for _, err := range e.Tick(ctx) {
			fmt.Fprintln(log, err)
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}

func (e *Engine) Tick(ctx context.Context) []error {
	e.mu.Lock()
	defer e.mu.Unlock()
	runs, err := e.db.scheduledRuns()
	if err != nil {
		return []error{err}
	}
	var failures []error
	for _, r := range runs {
		if err := ctx.Err(); err != nil {
			break
		}
		if err := e.step(ctx, &r); err != nil {
			if ctx.Err() != nil {
				break
			}
			failures = append(failures, fmt.Errorf("run %s: %w", r.ID, err))
			r.State, r.Detail = "blocked", err.Error()
			if err := e.db.save(&r, "blocked"); err != nil {
				failures = append(failures, err)
			}
		}
	}
	return failures
}

func (e *Engine) step(ctx context.Context, r *Run) error {
	if r.Landing != "" {
		c := candidateByID(r, r.Landing)
		if c == nil {
			return errors.New("landing candidate is missing")
		}
		head, err := project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
		if err != nil {
			return err
		}
		r.Landing = ""
		if head == c.Commit {
			e.markLanded(r, c)
			return e.db.save(r, "landing-recovered")
		}
		r.Held = true
		return e.db.save(r, "landing-needs-review")
	}
	workersBusy, controlBusy, chatBusy := false, false, false
	for i := range r.Turns {
		turn := &r.Turns[i]
		if turn.State != "running" {
			continue
		}
		if r.State == "stopped" {
			_ = stopJob(e.jobdir(*r, turn.ID))
		}
		result, received := reception(e.jobdir(*r, turn.ID))
		if !received {
			if turn.Role == "worker" {
				workersBusy = true
			} else if turn.Role == "chat" && turn.Generation == r.Generation {
				chatBusy = true
			} else if turn.Generation == r.Generation {
				controlBusy = true
			}
			continue
		}
		turn.State = "received"
		if turn.Role == "release" {
			return e.finishRelease(r, *turn, result)
		}
		if result.Error == "" && turn.Role == "worker" {
			if t := taskByID(r, turn.Task); t != nil && turn.Messages <= len(t.Notes) {
				t.Delivered = max(t.Delivered, turn.Messages)
			}
		}
		if turn.Role != "eval" {
			if result.Tokens == nil {
				r.UnknownTurns++
			} else {
				r.Tokens += *result.Tokens
			}
		}
		if r.State != "stopped" && (turn.Role == "worker" || turn.Role == "eval" || turn.Generation == r.Generation) {
			if turn.Role == "observe" {
				return e.finishObservation(r, *turn, result)
			}
			if err := e.accept(ctx, r, *turn, result); err != nil {
				return err
			}
		} else if turn.Role == "chat" && turn.Generation == r.Generation {
			e.finishChat(r, *turn, adapter.Result{Error: "Conversation paused when the run stopped."})
		} else if turn.Role == "eval" {
			if candidate := candidateByID(r, turn.Task); candidate != nil {
				candidate.State, candidate.Detail = "cancelled", "verification stopped"
			}
		} else if turn.Role == "worker" {
			if t := taskByID(r, turn.Task); t != nil {
				if result.Session != "" {
					t.Session = result.Session
				}
				t.State = "interrupted"
			}
		}
		if err := e.db.save(r, "received"); err != nil {
			return err
		}
	}
	if !chatBusy && !controlBusy && r.State != "stopped" && r.State != "planning" && r.State != "repairing" {
		if request := pendingMessage(r); request != nil {
			if r.Input.TokenTarget > 0 && r.Tokens >= r.Input.TokenTarget {
				request.State, request.Error = "failed", "The run reached its token target."
				return e.db.save(r, "conversation-paused")
			}
			return e.dispatchChat(r, request)
		}
	}
	if !chatBusy && !controlBusy && len(r.Observations) > 0 && r.ObserverError == "" && r.State != "stopped" &&
		(r.Input.TokenTarget == 0 || r.Tokens < r.Input.TokenTarget) && r.State != "planning" && r.State != "checks" && r.State != "repairing" {
		return e.observeTurn(r)
	}
	if err := e.releaseStep(ctx, r); err != nil {
		return err
	}
	if r.State == "review" {
		return e.maybeLand(ctx, r)
	}
	switch r.State {
	case "stopped", "landed", "blocked", "review", "question", "checks":
		return nil
	}
	if r.State == "evaluating" {
		return e.candidateStep(ctx, r)
	}
	if r.State == "working" && !workersBusy && allCommitted(r) {
		return e.candidates(ctx, r)
	}
	if r.Input.TokenTarget > 0 && r.Tokens >= r.Input.TokenTarget {
		if r.State != "budget" {
			r.State = "budget"
			return e.db.save(r, "token-target")
		}
		return nil
	}
	if r.State == "planning" && !controlBusy {
		return e.plan(ctx, r)
	}
	if r.State == "repairing" && !controlBusy {
		return e.repair(r)
	}
	if r.State == "working" {
		for i := r.TaskStart; i < len(r.Tasks); i++ {
			t := &r.Tasks[i]
			if t.State != "pending" || r.taskHeld(t.ID) || !depsReady(r, t) {
				continue
			}
			if err := e.worker(ctx, r, t); err != nil {
				return err
			}
		}
	}
	return nil
}

func taskByID(r *Run, id string) *Task {
	for i := range r.Tasks {
		if r.Tasks[i].ID == id {
			return &r.Tasks[i]
		}
	}
	return nil
}

func candidateByID(r *Run, id string) *Candidate {
	for i := range r.Candidates {
		if r.Candidates[i].ID == id {
			return &r.Candidates[i]
		}
	}
	return nil
}

func awaitingChildren(r *Run, id string) bool {
	for _, child := range r.activeTasks() {
		if child.Parent == id && child.State != "committed" && child.State != "abandoned" {
			return true
		}
	}
	return false
}

func depsReady(r *Run, t *Task) bool {
	if awaitingChildren(r, t.ID) {
		return false
	}
	for _, d := range t.Depends {
		if other := taskByID(r, d); other == nil || other.State != "committed" {
			return false
		}
	}
	return true
}

func allCommitted(r *Run) bool {
	for _, t := range r.activeTasks() {
		if t.State != "committed" && t.State != "abandoned" {
			return false
		}
	}
	return len(r.activeTasks()) > 0
}

func (e *Engine) plan(ctx context.Context, r *Run) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	files, _ := project.Files(ctx, e.integration(*r), r.workBase())
	if len(files) > 12000 {
		files = files[:12000]
	}
	var guidance strings.Builder
	for _, name := range []string{"AGENTS.md", "README.md", "package.json", "Makefile", "go.mod", "pyproject.toml", ".swarm.json"} {
		path := name
		if r.Repo.Subdir != "." {
			path = filepath.Join(r.Repo.Subdir, name)
		}
		if text, err := project.ReadBlob(ctx, e.integration(*r), r.workBase(), path); err == nil {
			if len(text) > 2000 {
				text = text[:2000]
			}
			fmt.Fprintf(&guidance, "\n[%s]\n%s\n", path, text)
		}
	}
	prompt := fmt.Sprintf(`You coordinate coding agents through Pantheon Local. Return only JSON:
{"tasks":[{"id":"short-slug","goal":"one coherent implementation milestone","worker":"fallback-profile-id","class":"implementation|debugging|testing|automation|documentation|design","depends":["prior-id"],"paths":["optional repo-relative files or folders"],"variant":"optional-approach-slug"}],"recipe":{"checks":[["program","arg"]],"preview":["program","arg"]},"question":""}
Use existing project conventions. If comparison is requested, propose competing hypotheses as alternatives only where worthwhile. Ask about what improvement and constraints matter if they are unclear. Suggest named cases and a selection rule when an independently runnable evaluator is available. An independent driver can be an absolute path the developer provided or a self-contained program string (set program and interpreter), with expected inputs and a JSON result { "pass": true, "observed": "actual behavior", "metrics": {} }. The developer approves that evaluator and selection rule. Compare requested: %t. For an empty repo consider a foundation followed by parallel milestones. Declare every prerequisite so independent milestones can run in parallel. Give each competing approach its own variant slug, shared by any milestones that implement it. Leave shared milestones without a variant. Aim to preserve compatibility. A major implementation milestone must end in a relevant Git commit. Propose an executable acceptance recipe, including dependency setup inside its commands when needed. You may ask one consequential question with no tasks. For each milestone choose its closest class. Pantheon Local may compare available profiles on the relevant benchmark, so always include a sensible fallback. Workers: %q.
Project subdirectory: %s. Committed files:
%s
Project guidance and build commands:
%s
Brief:
%s
Human clarification: %s
Round goal: %s
Previously used milestone IDs: %v`, r.Input.Compare, r.Input.Workers, r.Repo.Subdir, files, guidance.String(), r.Input.Prompt, r.Answer, r.RoundGoal, priorTaskIDs(r))
	return e.dispatch(r, "plan", "", job{Request: adapter.Request{Profile: r.Profiles[r.Input.Coordinator], Role: "coordinator", Dir: e.integration(*r), Prompt: prompt, Session: r.Session}})
}

func (e *Engine) worker(ctx context.Context, r *Run, t *Task) error {
	recovered := false
	if t.Workspace == "" {
		base := r.workBase()
		if err := e.followUpBase(ctx, r, t); err != nil {
			return err
		}
		if t.RepairBase != "" {
			base = t.RepairBase
		}
		workspace := filepath.Join(e.root(*r), "work", t.ID)
		if _, err := os.Stat(workspace); err == nil {
			workspace += "-" + uid()
		}
		if err := project.Clone(ctx, e.integration(*r), workspace, base, "swarm-"+t.ID); err != nil {
			return err
		}
		if err := project.ApplyCommitPolicy(ctx, r.Repo.Root, workspace); err != nil {
			return err
		}
		for _, id := range t.Donors {
			c := candidateByID(r, id)
			if c == nil {
				return errors.New("donor is missing")
			}
			if err := project.Fetch(ctx, workspace, e.integration(*r), c.Commit); err != nil {
				return err
			}
			if err := project.Pin(ctx, workspace, "refs/swarm/donors/"+id, c.Commit); err != nil {
				return err
			}
		}
		if t.Conflict != "" {
			c := candidateByID(r, t.Conflict)
			if c == nil || len(c.Pending) == 0 {
				return errors.New("conflict inputs are missing")
			}
			sha := taskByID(r, c.Pending[0]).Commit
			if err := project.Fetch(ctx, workspace, e.integration(*r), sha); err != nil {
				return err
			}
			mergeErr := project.Merge(ctx, workspace, sha)
			paths, err := project.ConflictedPaths(ctx, workspace)
			if err != nil || mergeErr == nil || len(paths) == 0 {
				return errors.New("could not reproduce integration conflict")
			}
		}
		for _, dependency := range t.Depends {
			sha := taskByID(r, dependency).Commit
			if err := project.Fetch(ctx, workspace, e.integration(*r), sha); err != nil {
				return err
			}
			if err := project.Merge(ctx, workspace, sha); err != nil {
				return err
			}
		}
		var err error
		t.Base, err = project.Head(ctx, workspace)
		if err != nil {
			return err
		}
		t.Workspace = workspace
	} else {
		if sha, err := project.Milestone(ctx, t.Workspace, t.Base); err == nil && t.Reply == "" && len(t.Notes) == t.Delivered {
			if err = e.saveCommit(ctx, r, t, sha); err != nil {
				return err
			}
			recovered = true
		}
	}
	if recovered {
		return e.db.save(r, "commit-recovered")
	}
	var delegated []string
	for _, child := range r.Tasks {
		if child.Parent != t.ID || child.Commit == "" {
			continue
		}
		if err := project.Fetch(ctx, t.Workspace, e.integration(*r), child.Commit); err != nil {
			return err
		}
		ref := "refs/swarm/delegated/" + child.ID
		if err := project.Pin(ctx, t.Workspace, ref, child.Commit); err != nil {
			return err
		}
		delegated = append(delegated, ref)
	}
	dir := workdir(t.Workspace, r.Repo.Subdir)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	prompt := fmt.Sprintf(`Implement this milestone in the assigned private checkout. Follow project instructions. Use native subagents if your harness supports them and the milestone has independent work worth delegating. Keep delegated work within this milestone and its allowed paths. Give concurrent writers separate files. Wait for all delegated work and review its results before finishing. You own the outcome and the Git commit. Run relevant checks, review the diff, stage only intended files and make a normal Git commit. Do not push. Return only JSON:
{"status":"committed|question|finding|blocked","commit":"sha when committed","message":"question, finding, or validation summary","recipient":"task ID when sending a finding"}
Brief: %s
Milestone (%s): %s
Allowed repo-relative paths (an empty list leaves scope to the milestone): %v
Applicable acceptance commands: %v
Human reply: %s
Messages: %s`, r.Input.Prompt, t.ID, t.Goal, t.Paths, r.Input.Recipe.Checks, t.Reply, strings.Join(t.Notes[t.Delivered:], "\n"))
	if len(t.Donors) > 0 {
		refs := make([]string, 0, len(t.Donors))
		for _, id := range t.Donors {
			refs = append(refs, "refs/swarm/donors/"+id)
		}
		prompt += fmt.Sprintf("\nInspect these local Git refs for the requested aspects: %v. Incorporate only requested behavior.", refs)
	}
	if len(delegated) > 0 {
		prompt += fmt.Sprintf("\nReview all delegated results at these local refs and integrate the requested work into your milestone commit: %v.", delegated)
	}
	if t.Conflict != "" {
		prompt += "\nResolve the Git merge already in progress, preserving the intended changes on both sides. Commit the merge."
	}
	t.State = "running"
	return e.dispatch(r, "worker", t.ID, job{Request: adapter.Request{Profile: r.Profiles[t.Worker], Role: "worker", Dir: dir, Prompt: prompt, Session: t.Session}})
}

func priorTaskIDs(r *Run) []string {
	ids := make([]string, 0, r.TaskStart)
	for _, t := range r.Tasks[:r.TaskStart] {
		ids = append(ids, t.ID)
	}
	return ids
}
