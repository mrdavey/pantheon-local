package run

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/project"
)

type job struct {
	Inputs  string      `json:"inputs,omitempty"`
	Release *releaseJob `json:"release,omitempty"`

	Scope          eval.Scope      `json:"scope"`
	ExpectedCommit string          `json:"expected_commit,omitempty"`
	Request        adapter.Request `json:"request"`
	Recipe         *eval.Recipe    `json:"recipe,omitempty"`
}

func writeJSON(path string, value any) error {
	temp, err := os.CreateTemp(filepath.Dir(path), ".swarm-*")
	if err != nil {
		return err
	}
	defer os.Remove(temp.Name())
	if err = json.NewEncoder(temp).Encode(value); err == nil {
		err = temp.Sync()
	}
	if closeErr := temp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(temp.Name(), path)
}

func readJSON(path string, value any) error {
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(raw, value)
}

func spawn(dir string) error {
	bin, err := os.Executable()
	if err != nil {
		return err
	}
	cmd := exec.Command(bin, "_supervise", dir)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	var spec job
	if err := readJSON(filepath.Join(dir, "job.json"), &spec); err != nil {
		return err
	}
	if spec.Release == nil {
		state := filepath.Join(dir, "..", "..", "..", "..")
		cmd.Env, err = workerEnvironment(state)
		if err != nil {
			return err
		}
	}
	if err = cmd.Start(); err != nil {
		return err
	}
	go func() { _ = cmd.Wait() }()
	return nil
}

func Supervise(ctx context.Context, dir string) error {
	claim, err := os.OpenFile(filepath.Join(dir, "claim"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	if _, err = fmt.Fprintln(claim, os.Getpid()); err != nil {
		_ = claim.Close()
		return err
	}
	_ = claim.Close()
	var spec job
	if err = readJSON(filepath.Join(dir, "job.json"), &spec); err != nil {
		return err
	}
	if _, err = os.Stat(filepath.Join(dir, "stop")); err == nil {
		return writeJSON(filepath.Join(dir, "result.json"), adapter.Result{Error: "stopped"})
	}
	log, err := os.OpenFile(filepath.Join(dir, "events.log"), os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer log.Close()
	var result adapter.Result
	if spec.Release != nil {
		result = superviseRelease(ctx, *spec.Release, log)
	} else if spec.Recipe != nil {
		err := project.CheckSnapshot(ctx, spec.Request.Dir, spec.ExpectedCommit)
		if err == nil {
			err = eval.Run(ctx, spec.Request.Dir, dir, *spec.Recipe, spec.Scope, log)
		}
		if len(spec.Recipe.Cases) > 0 {
			report := eval.Report{Commit: spec.ExpectedCommit, Inputs: spec.Inputs, Checks: "pass"}
			if err != nil {
				report.Checks = "failed"
			}
			inputs, measureErr := eval.Inputs(*spec.Recipe, spec.ExpectedCommit)
			if measureErr == nil && inputs != spec.Inputs {
				measureErr = errors.New("evaluation inputs changed before execution")
			}
			if measureErr == nil {
				deadline, cancel := context.WithTimeout(ctx, 20*time.Minute)
				report.Samples, measureErr = eval.Measure(deadline, spec.Request.Dir, filepath.Join(dir, "samples"), *spec.Recipe, spec.Scope, log)
				cancel()
			}
			if measureErr != nil {
				err = measureErr
			}
			if writeErr := writeJSON(filepath.Join(dir, "measurements.json"), report); writeErr != nil {
				err = writeErr
			}
		}
		if checkErr := project.CheckSnapshot(ctx, spec.Request.Dir, spec.ExpectedCommit); checkErr != nil {
			err = checkErr
		}
		if err != nil {
			result.Error = err.Error()
		}
	} else {
		var mu sync.Mutex
		encode := json.NewEncoder(log)
		result = adapter.Run(ctx, spec.Request, func(event adapter.Event) {
			mu.Lock()
			defer mu.Unlock()
			_ = encode.Encode(event)
		})
	}
	if err := log.Sync(); err != nil && result.Error == "" {
		result.Error = err.Error()
	}
	return writeJSON(filepath.Join(dir, "result.json"), result)
}

func reception(dir string) (adapter.Result, bool) {
	var result adapter.Result
	if err := readJSON(filepath.Join(dir, "result.json"), &result); err == nil {
		return result, true
	}
	fi, err := os.Stat(filepath.Join(dir, "job.json"))
	if err != nil {
		return adapter.Result{Error: "dispatch intent missing"}, true
	}
	if time.Since(fi.ModTime()) < 4*time.Second {
		return result, false
	}
	claim, err := os.Open(filepath.Join(dir, "claim"))
	if err == nil {
		defer claim.Close()
		var pid int
		if _, err = fmt.Fscan(claim, &pid); err == nil && pid > 1 && belongsToJob(pid, dir) {
			return result, false
		}
	}
	return adapter.Result{Error: "turn outcome uncertain; inspect the checkout and retry explicitly"}, true
}

func stopJob(dir string) error {
	file, err := os.OpenFile(filepath.Join(dir, "stop"), os.O_WRONLY|os.O_CREATE, 0600)
	if err != nil {
		return err
	}
	_ = file.Close()
	if _, err = os.Stat(filepath.Join(dir, "result.json")); err == nil {
		return nil
	}
	claim, err := os.Open(filepath.Join(dir, "claim"))
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	defer claim.Close()
	var pid int
	if _, err = fmt.Fscan(claim, &pid); err != nil || pid <= 1 {
		return errors.New("invalid process identity")
	}
	if !belongsToJob(pid, dir) {
		return nil
	}
	if err = syscall.Kill(-pid, syscall.SIGTERM); errors.Is(err, syscall.ESRCH) {
		return nil
	}
	return err
}

func belongsToJob(pid int, dir string) bool {
	if syscall.Kill(pid, 0) != nil {
		return false
	}
	raw, err := exec.Command("ps", "-p", fmt.Sprint(pid), "-o", "command=").Output()
	return err == nil && strings.HasSuffix(strings.TrimSpace(string(raw)), " _supervise "+dir)
}
