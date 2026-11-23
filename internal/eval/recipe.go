package eval

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

type Recipe struct {
	Cases  []Case `json:"cases,omitempty"`
	Rule   *Rule  `json:"rule,omitempty"`
	Rubric string `json:"rubric,omitempty"`

	Checks  [][]string `json:"checks"`
	Preview []string   `json:"preview,omitempty"`
}

func (r Recipe) Validate() error {
	if len(r.Checks) == 0 && len(r.Cases) == 0 && strings.TrimSpace(r.Rubric) == "" {
		return errors.New("supply at least one acceptance check")
	}
	for _, command := range r.Checks {
		if len(command) == 0 || strings.TrimSpace(command[0]) == "" {
			return errors.New("every check needs an executable")
		}
	}
	if len(r.Preview) > 0 && strings.TrimSpace(r.Preview[0]) == "" {
		return errors.New("preview needs an executable")
	}
	return r.validateComparison()
}

type Scope struct {
	Repository string `json:"repository,omitempty"`
	State      string `json:"state,omitempty"`
}

func Command(ctx context.Context, dir, data string, args []string, scope Scope) *exec.Cmd {
	return command(ctx, dir, data, args, scope, "")
}

func command(ctx context.Context, dir, data string, args []string, scope Scope, driver string) *exec.Cmd {
	if runtime.GOOS == "darwin" {
		args = append([]string{"/usr/bin/sandbox-exec", "-p", darwinPolicy(dir, data, scope, driver)}, args...)
	}
	cmd := exec.CommandContext(ctx, args[0], args[1:]...)
	cmd.Dir = dir
	for _, name := range []string{"PATH", "LANG", "LC_ALL", "TERM", "SystemRoot"} {
		if value, ok := os.LookupEnv(name); ok {
			cmd.Env = append(cmd.Env, name+"="+value)
		}
	}
	cmd.Env = append(cmd.Env, "HOME="+data, "TMPDIR="+data, "SWARM_DATA_DIR="+data)
	return cmd
}

func Run(ctx context.Context, dir, artifacts string, recipe Recipe, scope Scope, log *os.File) error {
	if err := recipe.Validate(); err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Minute)
	defer cancel()
	data := filepath.Join(artifacts, "data")
	if err := os.MkdirAll(data, 0700); err != nil {
		return err
	}
	for _, check := range recipe.Checks {
		fmt.Fprintf(log, "[check] %q\n", check)
		cmd := Command(ctx, dir, data, check, scope)
		cmd.Stdout, cmd.Stderr = log, log
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("%s: %w", check[0], err)
		}
	}
	return nil
}

// macOS limits writes to the disposable checkout and data directory. It also
// hides the developer's checkout, local service state, and common harness homes.
func darwinPolicy(dir, data string, scope Scope, driver string) string {
	quoted := func(path string) string {
		if absolute, err := filepath.EvalSymlinks(path); err == nil {
			path = absolute
		}
		return strconv.Quote(path)
	}
	work, scratch := quoted(dir), quoted(data)
	outside := fmt.Sprintf("(require-not (subpath %s)) (require-not (subpath %s))", work, scratch)
	var policy strings.Builder
	fmt.Fprintf(&policy, `(version 1) (allow default) (deny file-write* (require-all %s (require-not (literal "/dev/null")))) `, outside)
	block := []string{scope.Repository, scope.State}
	if home, err := os.UserHomeDir(); err == nil {
		block = append(block, filepath.Join(home, ".codex"), filepath.Join(home, ".claude"), filepath.Join(home, ".hermes"), filepath.Join(home, ".gemini"))
	}
	except := outside
	if driver != "" {
		except += fmt.Sprintf(" (require-not (literal %s))", quoted(driver))
	}
	for _, path := range block {
		if path != "" {
			fmt.Fprintf(&policy, "(deny file-read-data (require-all (subpath %s) %s)) ", quoted(path), except)
		}
	}
	return policy.String()
}
