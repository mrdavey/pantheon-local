package run

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/project"
)

func (e *Engine) pinRecipe(r *Run, recipe eval.Recipe) (eval.Recipe, error) {
	recipe.Cases = slices.Clone(recipe.Cases)
	for i := range recipe.Cases {
		c := &recipe.Cases[i]
		if c.Revision == 0 {
			continue
		}
		entry, err := e.db.caseRevision(r.Project, c.ID, c.Revision)
		if err != nil || entry.State == "retired" {
			return recipe, fmt.Errorf("case %s revision %d is not available", c.ID, c.Revision)
		}
		if len(c.Input) == 0 {
			c.Input, _ = json.Marshal(entry.Scenario)
		}
		if len(c.Expected) == 0 {
			c.Expected, _ = json.Marshal(entry.Expected)
		}
	}
	return eval.Pin(recipe, filepath.Join(e.root(*r), "evaluators"))
}

func (e *Engine) Evidence(runID, candidateID string) (eval.Report, error) {
	var report eval.Report
	r, err := e.Get(runID)
	if err != nil {
		return report, err
	}
	c := candidateByID(&r, candidateID)
	if c == nil || c.Evidence == "" {
		return report, errors.New("no measured evidence for this candidate")
	}
	err = readJSON(filepath.Join(e.jobdir(r, c.Evidence), "measurements.json"), &report)
	return report, err
}

func (e *Engine) recordReport(r *Run, c *Candidate, jobID string) error {
	var report eval.Report
	if err := readJSON(filepath.Join(e.jobdir(*r, jobID), "measurements.json"), &report); err != nil {
		return err
	}
	if report.Commit != c.Commit || report.Inputs != c.Inputs {
		return errors.New("evaluation receipt does not match the candidate inputs")
	}
	c.Evidence = jobID
	c.State, c.Detail = report.Outcome(r.Input.Recipe)
	if c.State == "inconclusive" {
		c.Score = nil
		return nil
	}
	samples := report.Samples
	if slices.ContainsFunc(r.Input.Recipe.Cases, func(c eval.Case) bool { return c.Holdback }) {
		held := map[string]bool{}
		for _, entry := range r.Input.Recipe.Cases {
			held[entry.ID] = entry.Holdback
		}
		samples = slices.DeleteFunc(slices.Clone(samples), func(s eval.Sample) bool { return !held[s.Case] })
	}
	c.Score = eval.Scores(samples, r.Input.Recipe.Rule)
	return nil
}

func (e *Engine) dispatchEvaluation(r *Run, c *Candidate, dir string) error {
	recipe := r.Input.Recipe
	if len(recipe.Cases) > 0 {
		var err error
		c.Inputs, err = eval.Inputs(recipe, c.Commit)
		if err != nil {
			c.State, c.Detail = "inconclusive", err.Error()
			return e.db.save(r, "eval-inputs-unavailable")
		}
		for _, prior := range r.Candidates {
			if prior.ID != c.ID && prior.State == "passed" && prior.Evidence != "" && prior.Inputs == c.Inputs {
				reused := *c
				if err := e.recordReport(r, &reused, prior.Evidence); err == nil && reused.State == "passed" {
					*c = reused
					c.Log = prior.Log
					r.observe("eval-complete", c.ID, "Reused matching measurements")
					return e.db.save(r, "eval-reused")
				}
			}
		}
	}
	c.State = "evaluating"
	return e.dispatch(r, "eval", c.ID, job{Request: adapter.Request{Dir: workdir(dir, r.Repo.Subdir)}, Recipe: &recipe,
		ExpectedCommit: c.Commit, Inputs: c.Inputs, Scope: eval.Scope{Repository: r.Repo.Root, State: e.dir}})
}

func metricDirection(rule *eval.Rule, name string) float64 {
	for _, m := range rule.Metrics {
		if m.Name == name && m.Direction == "lower" {
			return -1
		}
	}
	return 1
}

func eligible(rule *eval.Rule, c Candidate) bool {
	if c.State != "passed" || c.Baseline {
		return false
	}
	if rule == nil {
		return true
	}
	if _, ok := c.Score[rule.Objective]; !ok {
		return false
	}
	for name, limit := range rule.Limits {
		score, ok := c.Score[name]
		if !ok || (score.Value-limit)*metricDirection(rule, name) < 0 {
			return false
		}
	}
	return true
}

func missingMeasurements(rule *eval.Rule, c Candidate) bool {
	if rule == nil || c.State != "passed" {
		return false
	}
	if c.Score[rule.Objective].N == 0 {
		return true
	}
	for name := range rule.Limits {
		if c.Score[name].N == 0 {
			return true
		}
	}
	return false
}

func recommend(r *Run) Selection {
	selection := Selection{Round: r.Round, Policy: r.Policy, At: time.Now().UTC(), State: "undecided"}
	rule := r.Input.Recipe.Rule
	var candidates []Candidate
	var baseline *Candidate
	incomplete := false
	for _, c := range r.Candidates {
		if c.Round != r.Round || c.Policy != r.Policy {
			continue
		}
		if c.Baseline {
			baseline = &c
			continue
		}
		if c.State == "inconclusive" || c.State == "cancelled" || missingMeasurements(rule, c) {
			incomplete = true
		}
		if eligible(rule, c) {
			candidates = append(candidates, c)
		}
	}
	if incomplete {
		selection.Reason = "Some measurements are missing or inconclusive."
		return selection
	}
	if rule != nil && baseline != nil {
		base, ok := baseline.Score[rule.Objective]
		if baseline.State == "inconclusive" || !ok {
			selection.Reason = "The baseline could not be measured."
			return selection
		}
		candidates = slices.DeleteFunc(candidates, func(c Candidate) bool {
			gain := (c.Score[rule.Objective].Value - base.Value) * metricDirection(rule, rule.Objective)
			return gain <= 0 || gain < rule.Minimum
		})
	}
	if len(candidates) == 0 {
		selection.Reason = "No implementation met the approved rule."
		if baseline != nil {
			selection.State = "baseline"
		}
		return selection
	}
	if rule == nil && len(candidates) > 1 {
		selection.Reason = "Choose among the passing implementations."
		if r.Input.Recipe.Rubric != "" {
			selection.Reason = "Review the implementations against the rubric."
		}
		return selection
	}
	if rule != nil && len(candidates) > 1 {
		quality := func(c Candidate) float64 {
			return c.Score[rule.Objective].Value * metricDirection(rule, rule.Objective)
		}
		slices.SortStableFunc(candidates, func(a, b Candidate) int {
			if quality(a) > quality(b) {
				return -1
			}
			if quality(a) < quality(b) {
				return 1
			}
			if rule.TieBreak != "" {
				aScore, aOK := a.Score[rule.TieBreak]
				bScore, bOK := b.Score[rule.TieBreak]
				if aOK && bOK {
					diff := (aScore.Value - bScore.Value) * metricDirection(rule, rule.TieBreak)
					if diff > 0 {
						return -1
					}
					if diff < 0 {
						return 1
					}
				}
			}
			return 0
		})
		gap := quality(candidates[0]) - quality(candidates[1])
		// Every contender within the declared resolution must be compared.
		if gap == 0 || gap < rule.MinLead {
			best := candidates[0]
			betterTie := rule.TieBreak != "" && gap == 0 && rule.MinLead == 0
			for _, contender := range candidates[1:] {
				if quality(contender) != quality(best) {
					break
				}
				a, aOK := best.Score[rule.TieBreak]
				b, bOK := contender.Score[rule.TieBreak]
				betterTie = betterTie && aOK && bOK && (a.Value-b.Value)*metricDirection(rule, rule.TieBreak) > 0
			}
			if !betterTie {
				selection.Reason = "Leading implementations are tied or too close."
				return selection
			}
		}
	}
	selection.Recommended = candidates[0].ID
	selection.State = "recommended"
	selection.Reason = "All required checks passed."
	if rule != nil {
		selection.Reason = fmt.Sprintf("Qualified on %s (%.6g) and met the approved limits.", rule.Objective, candidates[0].Score[rule.Objective].Value)
	}
	if r.Input.Recipe.Rubric != "" {
		selection.State = "undecided"
		selection.Reason += " Review the implementations against the rubric before selecting."
	}
	return selection
}

func (e *Engine) selectCandidate(r *Run, cmd Command) error {
	if r.State != "review" || r.Landing != "" || r.Combination != nil {
		return errors.New("comparison is not open for selection")
	}
	next := recommend(r)
	next.Override = true
	if cmd.Kind == "keep-baseline" {
		next.State, next.Reason = "baseline", "You chose to keep the current version."
		r.Held = true
	} else {
		c := candidateByID(r, cmd.Candidate)
		if c == nil || c.Round != r.Round || c.Policy != r.Policy || !eligible(r.Input.Recipe.Rule, *c) {
			return errors.New("choose an eligible implementation from the current round")
		}
		next.State, next.Candidate, next.Reason = "selected", c.ID, "You selected this implementation."
	}
	e.archiveSelection(r)
	r.Selection = &next
	return nil
}

func (e *Engine) updateSuite(ctx context.Context, r *Run, cmd Command) error {
	if r.Landing != "" || r.State == "landed" || r.State == "planning" || r.State == "question" || r.State == "checks" || r.Combination != nil {
		return errors.New("the eval suite cannot change during this stage")
	}
	if slices.ContainsFunc(r.Turns, func(t Turn) bool { return t.State == "running" && t.Role == "eval" }) {
		return errors.New("wait for the current evaluation to finish")
	}
	recipe := cmd.Recipe
	if recipe == nil {
		recipe = r.SuiteProposal
	}
	if recipe == nil {
		return errors.New("supply a suite to approve")
	}
	pinned, err := e.pinRecipe(r, *recipe)
	if err != nil {
		return err
	}
	e.archiveSelection(r)
	r.Policy++
	r.Input.Recipe, r.SuiteProposal = pinned, nil
	r.notePolicy()
	if r.State == "review" || r.State == "evaluating" || r.State == "blocked" && allCommitted(r) {
		return e.candidates(ctx, r)
	}
	return nil
}

func (e *Engine) preference(r *Run, cmd Command) error {
	if cmd.Kind == "vote" {
		for i := range r.Preferences {
			p := &r.Preferences[i]
			if p.ID == cmd.Task && p.Choice == "" && p.Round == r.Round {
				if !slices.Contains([]string{"left", "right", "tie", "skip"}, cmd.Text) {
					return errors.New("choose A, B, tie, or skip")
				}
				p.Choice = cmd.Text
				reference := p.Recommended
				if (reference == p.Left || reference == p.Right) && (cmd.Text == "left" || cmd.Text == "right") {
					picked := p.Left
					if cmd.Text == "right" {
						picked = p.Right
					}
					match := picked == reference
					p.Agreement = &match
				}
				r.observe("preference", p.ID, cmd.Text)
				return nil
			}
		}
		return errors.New("this preference was already recorded or replaced")
	}
	if r.State != "review" || len(cmd.Donors) != 1 || cmd.Candidate == cmd.Donors[0] || strings.TrimSpace(r.Input.Recipe.Rubric) == "" {
		return errors.New("choose two reviewed candidates and an approved rubric")
	}
	for _, id := range []string{cmd.Candidate, cmd.Donors[0]} {
		c := candidateByID(r, id)
		if c == nil || c.Commit == "" || c.Round != r.Round || c.Policy != r.Policy {
			return errors.New("choose candidates from the current comparison")
		}
	}
	var coin [1]byte
	_, _ = rand.Read(coin[:])
	left, right := cmd.Candidate, cmd.Donors[0]
	if coin[0]%2 != 0 {
		left, right = right, left
	}
	recommended := ""
	if r.Selection != nil && r.Input.Recipe.Rule != nil {
		recommended = r.Selection.Recommended
	}
	r.Preferences = append(r.Preferences, Preference{ID: uid(), Round: r.Round, Left: left, Right: right, Rubric: r.Input.Recipe.Rubric, Recommended: recommended})
	return nil
}

func (e *Engine) PreferenceCandidate(runID, id, side string) (string, error) {
	r, err := e.Get(runID)
	if err != nil {
		return "", err
	}
	for _, p := range r.Preferences {
		if p.ID == id {
			if side == "a" {
				return p.Left, nil
			}
			if side == "b" {
				return p.Right, nil
			}
		}
	}
	return "", errors.New("unknown review side")
}

func (e *Engine) maybeLand(ctx context.Context, r *Run) error {
	if r.Input.Delivery == "" || r.Input.Delivery == "review" || r.Held || r.waitingForSelection() ||
		r.Combination != nil || r.ObserverError != "" || len(r.Observations) > 0 || r.Selection == nil {
		return nil
	}
	s := r.Selection
	if slices.ContainsFunc(r.Conversation, func(m Message) bool {
		return m.State == "queued" || m.State == "running" || m.ProposalState == "pending"
	}) {
		return nil
	}
	if s.State == "recommended" {
		s.Candidate, s.State = s.Recommended, "selected"
		if err := e.db.save(r, "selected"); err != nil {
			return err
		}
		return nil
	}
	if s.State != "selected" || s.Round != r.Round || s.Policy != r.Policy || r.Repo.Target == "" {
		return nil
	}
	c := candidateByID(r, s.Candidate)
	if c == nil {
		return nil
	}
	saved, err := e.land(ctx, *r, c.ID)
	if err != nil {
		if saved.Landing == "" && r.Detail == err.Error() {
			return nil
		}
		saved.Detail = err.Error()
		*r = saved
		return e.db.save(r, "landing-wait")
	}
	*r = saved
	return nil
}

func (e *Engine) repairConflict(ctx context.Context, r *Run, c *Candidate, source string, pending []string) error {
	conflicts, err := project.ConflictedPaths(ctx, source)
	if err != nil || len(conflicts) == 0 {
		c.State, c.Detail = "failed", "Git integration failed without a resolvable merge."
		return e.db.save(r, "assembly-failed")
	}
	head, err := project.Head(ctx, source)
	if err != nil {
		return err
	}
	c.State, c.Detail = "conflict", strings.Join(conflicts, ", ")
	c.ConflictPaths = slices.Clone(conflicts)
	for _, t := range r.Tasks {
		if t.Round == c.Round && t.Conflict == c.ID {
			return errors.New("integration repair already attempted")
		}
	}
	r.observe("integration-conflict", c.ID, c.Detail)
	// Keep the conflicted checkout private. The coordinator routes a new worker
	// via the normal repair turn so it owns the conflict decision and scope.
	c.Pending = slices.Clone(pending)
	c.ConflictHead = head
	if err := project.Import(ctx, e.integration(*r), source, head, "refs/swarm/conflicts/"+c.ID); err != nil {
		return err
	}
	return e.db.save(r, "assembly-conflict")
}

func (e *Engine) followUpBase(ctx context.Context, r *Run, t *Task) error {
	if len(t.Donors) == 0 || len(r.PastSelections) == 0 || r.Repo.Target == "" {
		return nil
	}
	last := r.PastSelections[len(r.PastSelections)-1]
	if last.LandedAt == nil {
		return nil
	}
	head, err := project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
	if err != nil {
		return err
	}
	if err := project.Fetch(ctx, e.integration(*r), r.Repo.Root, head); err != nil {
		return err
	}
	t.RepairBase = head
	return nil
}

func (e *Engine) report(r Run, c *Candidate) (eval.Report, error) {
	var report eval.Report
	err := readJSON(filepath.Join(e.jobdir(r, c.Evidence), "measurements.json"), &report)
	return report, err
}

func (e *Engine) measurementBusy(except string) (bool, error) {
	runs, err := e.db.scheduledRuns()
	if err != nil {
		return false, err
	}
	for _, r := range runs {
		if r.ID != except && len(r.Input.Recipe.Cases) > 0 &&
			slices.ContainsFunc(r.Turns, func(t Turn) bool { return t.Role == "eval" && t.State == "running" }) {
			return true, nil
		}
	}
	return false, nil
}

func (e *Engine) advanceConflict(ctx context.Context, r *Run, c *Candidate) error {
	for _, task := range r.Tasks {
		if task.Conflict != c.ID {
			continue
		}
		if task.State == "committed" {
			c.State = "superseded"
			r.Candidates = append(r.Candidates, Candidate{ID: uid(), Round: r.Round, Policy: r.Policy, Base: c.Base, Name: c.Name,
				Parents: []string{c.ID}, Tasks: append([]string{task.ID}, c.Pending[1:]...), State: "queued"})
			return e.db.save(r, "integration-repaired")
		}
		if task.State == "blocked" || task.State == "abandoned" {
			c.State, c.Detail = "failed", "The integration repair could not finish."
			return e.db.save(r, "integration-blocked")
		}
		if task.State == "pending" && !r.taskHeld(task.ID) && (r.Input.TokenTarget == 0 || r.Tokens < r.Input.TokenTarget) {
			return e.worker(ctx, r, taskByID(r, task.ID))
		}
		return nil
	}
	if len(r.Observations) == 0 && r.ObserverError == "" && !r.waitingForSelection() {
		c.State, c.Detail = "failed", "The coordinator did not route a repair."
		return e.db.save(r, "integration-blocked")
	}
	return nil
}
