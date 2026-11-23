package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/run"
	"github.com/mrdavey/pantheon-local/internal/service"
)

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := execute(ctx, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

type values []string

func (v *values) String() string         { return strings.Join(*v, ",") }
func (v *values) Set(value string) error { *v = append(*v, value); return nil }

func execute(ctx context.Context, args []string) error {
	if len(args) == 0 {
		args = []string{"open"}
	}
	if args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
		fmt.Println("swarm open|serve|doctor|start|status|answer|coordinator|chat|accept-proposal|approve|stop|resume|preview|diff|change|land|log|cases|history|evidence|destinations|feedback|answer-inbox|retry-coordinator|update-suite|select|keep-baseline|combine|new-round|cancel-combination|hold|unhold|delivery|blind-review|vote|hold-release|resume-release|reconcile-release")
		return nil
	}
	kind, args := args[0], args[1:]
	if kind == "_supervise" && len(args) == 1 {
		return run.Supervise(ctx, args[0])
	}
	dir, err := service.StateDir()
	if err != nil {
		return err
	}
	flags := flag.NewFlagSet(kind, flag.ContinueOnError)
	switch kind {
	case "open":
		if err := flags.Parse(args); err != nil {
			return err
		}
		if flags.NArg() != 0 {
			return errors.New("usage: swarm open")
		}
		return openWorkbench(ctx, dir)
	case "serve":
		listen := flags.String("listen", "127.0.0.1:0", "local HTTP address")
		if err := flags.Parse(args); err != nil {
			return err
		}
		return service.Serve(ctx, dir, *listen, os.Stdout, nil)
	case "doctor":
		cfg, err := service.ReadSetup(dir)
		if err != nil {
			return err
		}
		fmt.Println("Profiles:", filepath.Join(dir, "profiles.json"))
		for name, p := range cfg.Profiles {
			probe, cancel := context.WithTimeout(ctx, 5*time.Second)
			version, err := adapter.Probe(probe, p)
			cancel()
			if err != nil {
				fmt.Printf("%s: %v\n", name, err)
				continue
			}
			fmt.Printf("%s: %s\n", name, version)
		}
		return nil
	case "start":
		cfg, err := service.ReadSetup(dir)
		if err != nil {
			return err
		}
		var input run.Start
		var workers, files values
		flags.StringVar(&input.Path, "path", ".", "project folder")
		flags.StringVar(&input.Prompt, "prompt", "", "project brief")
		flags.StringVar(&input.Coordinator, "coordinator", cfg.Coordinator, "coordinator profile")
		flags.BoolVar(&input.Compare, "compare", false, "compare alternatives with the starting revision")
		flags.StringVar(&input.Delivery, "delivery", "review", "review, auto, or deploy")
		flags.StringVar(&input.Destination, "destination", "", "configured release destination")
		flags.StringVar(&input.Target, "target", "", "local target branch")
		flags.BoolVar(&input.SkipInit, "no-init", false, "require an existing commit")
		flags.Int64Var(&input.TokenTarget, "tokens", 0, "optional token target")
		flags.StringVar(&input.Routing, "routing", "", "manual, balanced, quality, or economy")
		flags.Var(&workers, "worker", "worker profile, repeatable")
		flags.Var(&files, "import", "initial relative file, repeatable")
		recipe := flags.String("recipe", "", "accepted JSON recipe")
		if err := flags.Parse(args); err != nil {
			return err
		}
		input.Path, err = filepath.Abs(input.Path)
		if err != nil {
			return err
		}
		input.InitialFiles = files
		input.Workers = workers
		if len(workers) == 0 {
			input.Workers = cfg.Workers
		}
		if *recipe != "" {
			v, err := readRecipe(*recipe)
			if err != nil {
				return err
			}
			input.Recipe = *v
		}
		return request(ctx, dir, "POST", "/api/runs", input)
	case "status":
		path := "/api/runs"
		if len(args) > 0 {
			path += "/" + args[0]
		}
		return request(ctx, dir, "GET", path, nil)
	case "destinations":
		return request(ctx, dir, "GET", "/api/destinations", nil)
	case "cases", "history":
		before := flags.Int64("before", 0, "case history cursor")
		if err := flags.Parse(args); err != nil {
			return err
		}
		if flags.NArg() != 1 {
			return fmt.Errorf("usage: swarm %s [--before cursor] <run-id>", kind)
		}
		return request(ctx, dir, "GET", fmt.Sprintf("/api/runs/%s/%s?before=%d", flags.Arg(0), kind, *before), nil)
	case "preview", "log", "diff", "evidence":
		if len(args) != 2 {
			return fmt.Errorf("usage: swarm %s <run-id> <id>", kind)
		}
		if kind == "evidence" {
			return request(ctx, dir, "GET", "/api/runs/"+args[0]+"/evidence/"+args[1], nil)
		}
		if kind == "diff" {
			return request(ctx, dir, "GET", "/api/runs/"+args[0]+"/diff/"+args[1], nil)
		}
		if kind == "log" {
			return request(ctx, dir, "GET", "/api/runs/"+args[0]+"/turns/"+args[1], nil)
		}
		return request(ctx, dir, "POST", "/api/runs/"+args[0]+"/preview/"+args[1], nil)
	case "answer", "coordinator", "chat", "accept-proposal", "dismiss-proposal", "approve", "stop", "resume", "change", "land", "feedback", "answer-inbox", "retry-coordinator", "update-suite", "select", "keep-baseline", "combine", "new-round", "cancel-combination",
		"hold", "unhold", "delivery", "blind-review", "vote", "hold-release", "resume-release", "reconcile-release":
		var action run.Command
		action.Kind = kind
		var donors values
		flags.Var(&donors, "donor", "another candidate ID, repeatable")
		flags.StringVar(&action.ID, "id", "", "idempotent command ID")
		revision := flags.Int64("revision", -1, "expected run revision")
		flags.StringVar(&action.Task, "task", "", "task ID")
		flags.StringVar(&action.Text, "text", "", "answer or profile")
		flags.StringVar(&action.Candidate, "candidate", "", "candidate ID")
		flags.Int64Var(&action.Tokens, "tokens", 0, "token target")
		recipe := flags.String("recipe", "", "accepted JSON recipe")
		if err := flags.Parse(args); err != nil {
			return err
		}
		if flags.NArg() != 1 {
			return fmt.Errorf("usage: swarm %s [flags] <run-id>", kind)
		}
		if *recipe != "" {
			v, err := readRecipe(*recipe)
			if err != nil {
				return err
			}
			action.Recipe = v
		}
		action.Donors = donors
		if *revision >= 0 {
			action.Expected = revision
		}
		return request(ctx, dir, "POST", "/api/runs/"+flags.Arg(0), action)
	default:
		return fmt.Errorf("unknown command %q", kind)
	}
}

func readRecipe(path string) (*eval.Recipe, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var recipe eval.Recipe
	if err = json.Unmarshal(raw, &recipe); err != nil {
		return nil, err
	}
	return &recipe, recipe.Validate()
}

func request(ctx context.Context, dir, method, path string, value any) error {
	raw, err := os.ReadFile(filepath.Join(dir, "service.json"))
	if err != nil {
		return fmt.Errorf("start the local service with swarm serve: %w", err)
	}
	var info service.Info
	if err = json.Unmarshal(raw, &info); err != nil {
		return err
	}
	body, err := json.Marshal(value)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, method, info.URL+path, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+info.Token)
	req.Header.Set("Content-Type", "application/json")
	client := http.Client{Timeout: 45 * time.Second}
	response, err := client.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode >= 400 {
		var message struct{ Error string }
		if json.NewDecoder(response.Body).Decode(&message) == nil && message.Error != "" {
			return errors.New(message.Error)
		}
		return errors.New(response.Status)
	}
	result, err := io.ReadAll(response.Body)
	if err != nil {
		return err
	}
	var indented bytes.Buffer
	if json.Indent(&indented, result, "", "  ") == nil {
		fmt.Println(indented.String())
		return nil
	}
	fmt.Print(string(result))
	return nil
}
