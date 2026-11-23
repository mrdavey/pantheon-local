package run

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/mrdavey/pantheon-local/internal/adapter"
)

func TestSchedulerDispatchesEveryReadyMilestone(t *testing.T) {
	ctx := t.Context()
	repo := t.TempDir()
	for _, args := range [][]string{{"init", "-b", "main"}, {"config", "user.name", "Scheduler Test"}, {"config", "user.email", "scheduler@example.invalid"}} {
		cmd := exec.CommandContext(ctx, "git", append([]string{"-C", repo}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %s (%v)", args, out, err)
		}
	}
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	var launched []job
	e.launch = func(dir string) error {
		var spec job
		if err := readJSON(filepath.Join(dir, "job.json"), &spec); err != nil {
			return err
		}
		launched = append(launched, spec)
		return nil
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	profiles := map[string]adapter.Profile{"worker": {Kind: "custom", Executable: executable}}
	r, err := e.Start(ctx, Start{Path: repo, Prompt: "Implement independent features", Coordinator: "worker", Workers: []string{"worker"}}, profiles)
	if err != nil {
		t.Fatal(err)
	}
	plan := Plan{}
	for i := range 30 {
		task := Task{ID: fmt.Sprintf("task-%d", i), Goal: "Implement a feature", Worker: "worker"}
		if i >= 4 {
			task.Depends = []string{"task-0"}
		}
		plan.Tasks = append(plan.Tasks, task)
	}
	if err := validatePlan(plan, r.Input.Workers); err != nil {
		t.Fatal(err)
	}
	r.Tasks, r.State = plan.Tasks, "working"
	e.assign(&r, r.Tasks)
	if err := e.step(ctx, &r); err != nil {
		t.Fatal(err)
	}
	if len(launched) != 4 {
		t.Fatalf("launched %d of 4 ready milestones", len(launched))
	}
	checkouts := make(map[string]bool)
	for _, spec := range launched {
		if spec.Request.Role != "worker" || checkouts[spec.Request.Dir] || spec.Request.Dir == repo {
			t.Fatalf("expected independent worker checkouts: %+v", spec.Request)
		}
		checkouts[spec.Request.Dir] = true
		if !strings.Contains(spec.Request.Prompt, "Use native subagents") || !strings.Contains(spec.Request.Prompt, "Wait for all delegated work") {
			t.Fatal("worker prompt did not authorize scoped, joined delegation")
		}
	}
	for _, task := range r.Tasks[4:] {
		if task.State != "pending" || task.Workspace != "" {
			t.Fatalf("dependent task dispatched early: %+v", task)
		}
	}
}

func TestSchedulerWakesLandedRunsAfterRestart(t *testing.T) {
	dir := t.TempDir()
	e, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, r := range []Run{
		{ID: "idle", Project: "project", State: "landed"}, {ID: "paused", State: "stopped"},
		{ID: "release", State: "landed", Experiment: Experiment{Releases: []Release{{State: "reconcile"}}}},
		{ID: "stopping", State: "stopped", Turns: []Turn{{State: "running"}}},
	} {
		if err := e.db.save(&r, "seed"); err != nil {
			t.Fatal(err)
		}
	}
	e.Close()
	e, err = Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	check := func(want string) {
		t.Helper()
		runs, err := e.db.scheduledRuns()
		if err != nil {
			t.Fatal(err)
		}
		var ids []string
		for _, r := range runs {
			ids = append(ids, r.ID)
		}
		if got := strings.Join(ids, ","); got != want {
			t.Fatalf("scheduled %s, want %s", got, want)
		}
	}
	check("stopping,release")
	check("stopping,release")
	if _, err := e.Command(t.Context(), "idle", Command{Kind: "feedback", Text: "Try another case"}); err != nil {
		t.Fatal(err)
	}
	check("stopping,release,idle")
	r, err := e.Get("idle")
	if err != nil {
		t.Fatal(err)
	}
	r.Observations = nil
	if err := e.db.save(&r, "invalid", EvalCase{ID: "case", Revision: 4}); err == nil {
		t.Fatal("invalid write committed")
	}
	check("stopping,release,idle")
	if err := e.db.save(&r, "received"); err != nil {
		t.Fatal(err)
	}
	check("stopping,release")
}

func BenchmarkIdleHistoryTick(b *testing.B) {
	for _, size := range []int{1, 100, 500} {
		b.Run(fmt.Sprint(size), func(b *testing.B) {
			e, err := Open(b.TempDir())
			if err != nil {
				b.Fatal(err)
			}
			defer e.Close()
			for i := range size {
				r := Run{ID: fmt.Sprint(i), State: "landed"}
				for j := range 30 {
					r.Tasks = append(r.Tasks, Task{ID: fmt.Sprint(j), State: "committed", Goal: "Implement and verify this milestone", Commit: "0123456789012345678901234567890123456789"})
					r.Turns = append(r.Turns, Turn{ID: fmt.Sprint(j), Role: "worker", State: "received"}, Turn{ID: fmt.Sprint(j + 30), Role: "observe", State: "received"})
				}
				if err := e.db.save(&r, "landed"); err != nil {
					b.Fatal(err)
				}
			}
			if failures := e.Tick(b.Context()); len(failures) > 0 {
				b.Fatal(failures)
			}
			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				if failures := e.Tick(b.Context()); len(failures) > 0 {
					b.Fatal(failures)
				}
			}
		})
	}
}
