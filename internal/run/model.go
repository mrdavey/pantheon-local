package run

import (
	"errors"
	"fmt"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/project"
	"github.com/mrdavey/pantheon-local/internal/routing"
)

type Start struct {
	Compare     bool   `json:"compare,omitempty"`
	Delivery    string `json:"delivery,omitempty"`
	Destination string `json:"destination,omitempty"`

	Routing        string      `json:"routing,omitempty"`
	OriginalPrompt string      `json:"original_prompt,omitempty"`
	Path           string      `json:"path"`
	Prompt         string      `json:"prompt"`
	Coordinator    string      `json:"coordinator"`
	Workers        []string    `json:"workers"`
	InitialFiles   []string    `json:"initial_files,omitempty"`
	SkipInit       bool        `json:"skip_init,omitempty"`
	Target         string      `json:"target,omitempty"`
	TokenTarget    int64       `json:"token_target,omitempty"`
	Recipe         eval.Recipe `json:"recipe"`
}

type Task struct {
	Variant  string   `json:"variant,omitempty"`
	Parent   string   `json:"parent,omitempty"`
	Donors   []string `json:"donors,omitempty"`
	Round    int      `json:"round,omitempty"`
	Conflict string   `json:"conflict,omitempty"`

	Class       string            `json:"class,omitempty"`
	Route       *routing.Decision `json:"route,omitempty"`
	Paths       []string          `json:"paths,omitempty"`
	Delivered   int               `json:"delivered,omitempty"`
	ID          string            `json:"id"`
	Goal        string            `json:"goal"`
	Worker      string            `json:"worker"`
	Depends     []string          `json:"depends,omitempty"`
	Alternative string            `json:"alternative,omitempty"`
	State       string            `json:"state,omitempty"`
	Workspace   string            `json:"workspace,omitempty"`
	Base        string            `json:"base,omitempty"`
	Commit      string            `json:"commit,omitempty"`
	Session     string            `json:"session,omitempty"`
	Question    string            `json:"question,omitempty"`
	Reply       string            `json:"reply,omitempty"`
	Finding     string            `json:"finding,omitempty"`
	RepairBase  string            `json:"repair_base,omitempty"`
	Notes       []string          `json:"notes,omitempty"`
}

type Plan struct {
	Tasks    []Task      `json:"tasks"`
	Recipe   eval.Recipe `json:"recipe"`
	Question string      `json:"question,omitempty"`
}

type Candidate struct {
	ConflictPaths []string `json:"conflict_paths,omitempty"`
	Eligible      *bool    `json:"eligible,omitempty"`
	Preview       []string `json:"preview,omitempty"`

	Pending      []string `json:"pending,omitempty"`
	ConflictHead string   `json:"conflict_head,omitempty"`

	Name     string                  `json:"name,omitempty"`
	Round    int                     `json:"round,omitempty"`
	Baseline bool                    `json:"baseline,omitempty"`
	Parents  []string                `json:"parents,omitempty"`
	Score    map[string]eval.Summary `json:"score,omitempty"`
	Evidence string                  `json:"evidence,omitempty"`
	Inputs   string                  `json:"inputs,omitempty"`
	Policy   int                     `json:"policy,omitempty"`

	ID     string   `json:"id"`
	Base   string   `json:"base"`
	Commit string   `json:"commit,omitempty"`
	Tasks  []string `json:"tasks"`
	State  string   `json:"state"`
	Detail string   `json:"detail,omitempty"`
	Log    string   `json:"log,omitempty"`
}

type Turn struct {
	Policy int `json:"policy,omitempty"`
	Round  int `json:"round,omitempty"`

	Messages   int    `json:"messages,omitempty"`
	ID         string `json:"id"`
	Task       string `json:"task,omitempty"`
	Role       string `json:"role"`
	Generation int    `json:"generation"`
	State      string `json:"state"`
}

type Run struct {
	StoppedFrom string            `json:"stopped_from,omitempty"`
	Project     string            `json:"project,omitempty"`
	Revision    int64             `json:"revision,omitempty"`
	Commands    map[string]string `json:"commands,omitempty"`
	Experiment

	Conversation     []Message                  `json:"conversation,omitempty"`
	ChatSession      string                     `json:"chat_session,omitempty"`
	BenchmarkNote    string                     `json:"benchmark_note,omitempty"`
	BenchmarkFetched string                     `json:"benchmark_fetched,omitempty"`
	RepairID         string                     `json:"repair_id,omitempty"`
	RevisionGoal     string                     `json:"revision_goal,omitempty"`
	Landing          string                     `json:"landing,omitempty"`
	ID               string                     `json:"id"`
	Input            Start                      `json:"input"`
	Repo             project.Repository         `json:"repo"`
	Profiles         map[string]adapter.Profile `json:"profiles"`
	State            string                     `json:"state"`
	Generation       int                        `json:"generation"`
	Session          string                     `json:"session,omitempty"`
	Tasks            []Task                     `json:"tasks"`
	Turns            []Turn                     `json:"turns"`
	Candidates       []Candidate                `json:"candidates"`
	RecipeApproved   bool                       `json:"recipe_approved"`
	Proposed         eval.Recipe                `json:"proposed_recipe"`
	Question         string                     `json:"question,omitempty"`
	Answer           string                     `json:"answer,omitempty"`
	Detail           string                     `json:"detail,omitempty"`
	Tokens           int64                      `json:"tokens"`
	UnknownTurns     int                        `json:"unknown_turns"`
}

type WorkerResult struct {
	Status    string `json:"status"`
	Commit    string `json:"commit,omitempty"`
	Message   string `json:"message,omitempty"`
	Recipient string `json:"recipient,omitempty"`
}

type Repair struct {
	Task        string `json:"task"`
	Instruction string `json:"instruction"`
}

var idPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,47}$`)

func validatePlan(plan Plan, workers []string) error {
	if plan.Question != "" && len(plan.Tasks) == 0 {
		return nil
	}
	return validateTasks(plan.Tasks, nil, workers)
}

func validateTasks(added, existing []Task, workers []string) error {
	if len(added) == 0 {
		return errors.New("supply at least one milestone")
	}
	workerSet := map[string]bool{}
	for _, worker := range workers {
		workerSet[worker] = true
	}
	tasks := map[string]Task{}
	for _, task := range existing {
		tasks[task.ID] = task
	}
	for _, task := range added {
		if !idPattern.MatchString(task.ID) || tasks[task.ID].ID != "" || !workerSet[task.Worker] || strings.TrimSpace(task.Goal) == "" {
			return fmt.Errorf("invalid milestone %q", task.ID)
		}
		if !routing.ClassValid(task.Class) {
			return fmt.Errorf("invalid benchmark category %q", task.Class)
		}
		if task.Variant != "" && (!idPattern.MatchString(task.Variant) || task.Alternative != "") {
			return errors.New("use a variant slug or a legacy alternative group")
		}
		if task.Alternative != "" && !idPattern.MatchString(task.Alternative) {
			return errors.New("invalid alternative group")
		}
		for _, path := range task.Paths {
			if !filepath.IsLocal(path) || filepath.Clean(path) == "." {
				return fmt.Errorf("invalid repo-relative milestone path %q", path)
			}
		}
		if task.Parent != "" || task.Round != 0 || task.Conflict != "" || len(task.Donors) != 0 || task.Route != nil || task.Delivered != 0 || task.State != "" || task.Workspace != "" || task.Commit != "" || task.Session != "" || task.Base != "" ||
			task.Question != "" || task.Reply != "" || task.Finding != "" || task.RepairBase != "" || len(task.Notes) != 0 {
			return errors.New("coordinator supplied service-owned milestone fields")
		}
		tasks[task.ID] = task
	}
	visited := map[string]int{}
	var visit func(string) error
	visit = func(id string) error {
		switch visited[id] {
		case 1:
			return fmt.Errorf("dependency cycle at %s", id)
		case 2:
			return nil
		}
		visited[id] = 1
		for _, dep := range tasks[id].Depends {
			if tasks[dep].ID == "" || tasks[dep].Alternative != "" ||
				(tasks[dep].Variant != "" && tasks[dep].Variant != tasks[id].Variant) {
				return fmt.Errorf("unavailable dependency %q", dep)
			}
			if err := visit(dep); err != nil {
				return err
			}
		}
		visited[id] = 2
		return nil
	}
	for id := range tasks {
		if err := visit(id); err != nil {
			return err
		}
	}
	return nil
}

func allowedPath(scopes []string, path string) bool {
	for _, scope := range scopes {
		scope = filepath.ToSlash(filepath.Clean(scope))
		if path == scope || strings.HasPrefix(path, scope+"/") {
			return true
		}
	}
	return len(scopes) == 0
}

func workdir(root, subdir string) string {
	return filepath.Join(root, subdir)
}
