package run

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/project"
)

type Destination struct {
	Executable  string   `json:"executable"`
	Environment []string `json:"environment,omitempty"`
	Rollback    bool     `json:"rollback,omitempty"`
	Digest      string   `json:"digest,omitempty"`
}

type Release struct {
	LastEffect  string     `json:"last_effect,omitempty"`
	ID          string     `json:"id"`
	Candidate   string     `json:"candidate"`
	Commit      string     `json:"commit"`
	Destination string     `json:"destination"`
	State       string     `json:"state"`
	Digest      string     `json:"digest,omitempty"`
	Previous    string     `json:"previous,omitempty"`
	Observed    string     `json:"observed,omitempty"`
	Detail      string     `json:"detail,omitempty"`
	Held        bool       `json:"held,omitempty"`
	At          *time.Time `json:"at,omitempty"`
}

type releaseJob struct {
	Repo        string
	Target      string
	Config      Destination
	Phase       string
	Operation   string
	Destination string
	Commit      string
	Source      string
	Artifact    string
	Digest      string
	Previous    string
}

type releaseReceipt struct {
	Digest  string `json:"digest"`
	Healthy *bool  `json:"healthy,omitempty"`
}

func readReleaseReceipt(raw []byte) (releaseReceipt, error) {
	var wire struct {
		Digest  *string `json:"digest"`
		Healthy *bool   `json:"healthy"`
	}
	if err := json.Unmarshal(raw, &wire); err != nil {
		return releaseReceipt{}, err
	}
	if wire.Digest == nil || wire.Healthy == nil {
		return releaseReceipt{}, errors.New("readback must report a digest and health")
	}
	return releaseReceipt{Digest: *wire.Digest, Healthy: wire.Healthy}, nil
}

func workerEnvironment(state string) ([]string, error) {
	var config map[string]Destination
	if err := readJSON(filepath.Join(state, "destinations.json"), &config); err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	omit := map[string]bool{}
	for _, destination := range config {
		for _, name := range destination.Environment {
			omit[name] = true
		}
	}
	return slices.DeleteFunc(os.Environ(), func(line string) bool {
		name, _, _ := strings.Cut(line, "=")
		return omit[name]
	}), nil
}

func (e *Engine) Destinations() ([]string, error) {
	var config map[string]Destination
	if err := readJSON(filepath.Join(e.dir, "destinations.json"), &config); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return []string{}, nil
		}
		return nil, err
	}
	names := []string{}
	for name := range config {
		if idPattern.MatchString(name) {
			names = append(names, name)
		}
	}
	slices.Sort(names)
	return names, nil
}

func digestFile(path string) (string, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return "", err
	}
	if !info.Mode().IsRegular() {
		return "", errors.New("expected a regular artifact")
	}
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return "sha256:" + hex.EncodeToString(h.Sum(nil)), nil
}

func (e *Engine) destination(name string) (Destination, error) {
	var config map[string]Destination
	if !idPattern.MatchString(name) {
		return Destination{}, errors.New("choose a configured destination")
	}
	if err := readJSON(filepath.Join(e.dir, "destinations.json"), &config); err != nil {
		return Destination{}, err
	}
	d, ok := config[name]
	if !ok || !filepath.IsAbs(d.Executable) {
		return d, errors.New("destination needs an absolute executable in destinations.json")
	}
	path, err := filepath.EvalSymlinks(d.Executable)
	if err != nil {
		return d, err
	}
	d.Executable = path
	if _, err := exec.LookPath(path); err != nil {
		return d, err
	}
	d.Digest, err = digestFile(path)
	return d, err
}

func (e *Engine) validateDelivery(input Start) error {
	switch input.Delivery {
	case "", "review", "auto":
		return nil
	case "deploy":
		_, err := e.destination(input.Destination)
		return err
	default:
		return errors.New("choose review, auto-land, or auto-deploy")
	}
}

func (e *Engine) authorizeDestination(r *Run) error {
	if r.Input.Delivery != "deploy" {
		return nil
	}
	d, err := e.destination(r.Input.Destination)
	if err != nil {
		return err
	}
	r.DeployConfig = &d
	return nil
}

func releaseCommandRun(ctx context.Context, j releaseJob, phase string, log io.Writer) (releaseReceipt, error) {
	var receipt releaseReceipt
	raw, err := json.Marshal(struct {
		Operation   string `json:"operation"`
		Destination string `json:"destination"`
		Commit      string `json:"commit"`
		Source      string `json:"source,omitempty"`
		Artifact    string `json:"artifact,omitempty"`
		Digest      string `json:"digest,omitempty"`
		Previous    string `json:"previous,omitempty"`
	}{j.Operation, j.Destination, j.Commit, j.Source, j.Artifact, j.Digest, j.Previous})
	if err != nil {
		return receipt, err
	}
	cmd := exec.CommandContext(ctx, j.Config.Executable, phase)
	cmd.Stdin = bytes.NewReader(raw)
	cmd.Stderr = log
	if j.Source != "" {
		cmd.Dir = j.Source
	}
	cmd.Env = []string{"PATH=" + os.Getenv("PATH"), "LANG=" + os.Getenv("LANG")}
	if phase != "build" && phase != "verify" {
		for _, name := range j.Config.Environment {
			if v, ok := os.LookupEnv(name); ok && !strings.Contains(name, "=") {
				cmd.Env = append(cmd.Env, name+"="+v)
			}
		}
	}
	var output bytes.Buffer
	cmd.Stdout = io.Discard
	if phase == "status" {
		cmd.Stdout = &output
	}
	if err := cmd.Run(); err != nil {
		return receipt, err
	}
	if phase == "build" || phase == "verify" || phase == "deploy" || phase == "rollback" {
		return receipt, nil
	}
	return readReleaseReceipt(output.Bytes())
}

func superviseRelease(parent context.Context, j releaseJob, log io.Writer) adapter.Result {
	ctx, cancel := context.WithTimeout(parent, 20*time.Minute)
	defer cancel()
	fail := func(err error) adapter.Result { return adapter.Result{Error: err.Error()} }
	if digest, err := digestFile(j.Config.Executable); err != nil || digest != j.Config.Digest {
		return fail(errors.New("authorized release adapter changed"))
	}
	switch j.Phase {
	case "build":
		if err := project.CheckSnapshot(ctx, j.Source, j.Commit); err != nil {
			return fail(err)
		}
		before, err := project.Head(ctx, j.Source)
		if err != nil || before != j.Commit {
			return fail(errors.New("release source does not match the tested commit"))
		}
		if _, err := releaseCommandRun(ctx, j, "build", log); err != nil {
			return fail(err)
		}
		if err := project.CheckSnapshot(ctx, j.Source, j.Commit); err != nil {
			return fail(err)
		}
		digest, err := digestFile(j.Artifact)
		if err != nil {
			return fail(err)
		}
		if err := os.Chmod(j.Artifact, 0400); err != nil {
			return fail(err)
		}
		j.Digest = digest
		if _, err := releaseCommandRun(ctx, j, "verify", log); err != nil {
			return fail(err)
		}
		if after, err := digestFile(j.Artifact); err != nil || after != j.Digest {
			return fail(errors.New("artifact changed during verification"))
		}
		return adapter.Result{Text: digest}
	case "readback":
		receipt, err := releaseCommandRun(ctx, j, "status", log)
		if err != nil {
			return fail(err)
		}
		raw, err := json.Marshal(receipt)
		if err != nil {
			return fail(err)
		}
		return adapter.Result{Text: string(raw)}
	case "deploy", "rollback":
		current, err := releaseCommandRun(ctx, j, "status", log)
		if err != nil {
			return fail(err)
		}
		expected := j.Previous
		if j.Phase == "rollback" {
			expected = j.Digest
		}
		if current.Digest != expected {
			return fail(errors.New("live artifact moved before release"))
		}
		if j.Phase == "deploy" {
			head, err := project.TargetHead(ctx, j.Repo, j.Target)
			if err != nil || head != j.Commit {
				return fail(errors.New("target moved before deployment"))
			}
			digest, err := digestFile(j.Artifact)
			if err != nil || digest != j.Digest {
				return fail(errors.New("release artifact changed"))
			}
		}
		if _, err := releaseCommandRun(ctx, j, j.Phase, log); err != nil {
			return fail(err)
		}
		return adapter.Result{}
	default:
		return fail(errors.New("unknown release phase"))
	}
}

func (e *Engine) releaseStep(ctx context.Context, r *Run) error {
	if len(r.Releases) == 0 {
		return nil
	}
	for i := range r.Releases {
		entry := &r.Releases[i]
		if r.State == "stopped" || entry.Held || r.DeployConfig == nil {
			continue
		}
		phase := ""
		switch entry.State {
		case "queued":
			phase = "build"
		case "built", "verify", "verify-rollback", "reconcile":
			phase = "readback"
		case "staged":
			phase = "deploy"
		case "needs-rollback":
			phase = "rollback"
		default:
			continue
		}
		runs, err := e.db.list()
		if err != nil {
			return err
		}
		for _, other := range runs {
			for _, current := range other.Releases {
				if current.ID != entry.ID && current.Destination == entry.Destination &&
					slices.Contains([]string{"reading-baseline", "staged", "deploying", "verify", "verifying", "needs-rollback", "rolling-back", "verify-rollback", "checking-rollback", "uncertain", "reconcile", "reconciling"}, current.State) &&
					(current.State != "staged" || entry.State == "built") {
					return nil
				}
			}
		}
		dir := filepath.Join(e.root(*r), "releases", entry.ID)
		source := filepath.Join(dir, "source")
		if phase == "deploy" || phase == "build" {
			head, err := project.TargetHead(ctx, r.Repo.Root, r.Repo.Target)
			if err != nil {
				return err
			}
			if head != entry.Commit {
				entry.State, entry.Detail = "superseded", "The target moved before promotion."
				return e.db.save(r, "release-superseded")
			}
		}
		switch entry.State {
		case "queued":
			if err := os.MkdirAll(dir, 0700); err != nil {
				return err
			}
			if _, err := os.Stat(source); err == nil {
				if err := os.RemoveAll(source); err != nil {
					return err
				}
			}
			if err := project.Clone(ctx, e.integration(*r), source, entry.Commit, "swarm-release"); err != nil {
				return err
			}
			entry.State = "building"
		case "built":
			entry.State = "reading-baseline"
		case "staged":
			entry.State, entry.LastEffect = "deploying", "deploy"
		case "verify":
			entry.State = "verifying"
		case "needs-rollback":
			entry.State, entry.LastEffect = "rolling-back", "rollback"
		case "verify-rollback":
			entry.State = "checking-rollback"
		case "reconcile":
			entry.State = "reconciling"
		}
		j := releaseJob{Config: *r.DeployConfig, Phase: phase, Operation: entry.ID + "/" + phase, Destination: entry.Destination, Commit: entry.Commit, Artifact: filepath.Join(dir, "artifact"),
			Digest: entry.Digest, Previous: entry.Previous, Repo: r.Repo.Root, Target: r.Repo.Target}
		if phase == "build" {
			j.Source = workdir(source, r.Repo.Subdir)
		}
		return e.dispatch(r, "release", entry.ID, job{Release: &j})
	}
	return nil
}

func (e *Engine) finishRelease(r *Run, turn Turn, result adapter.Result) error {
	for i := range r.Releases {
		entry := &r.Releases[i]
		if entry.ID != turn.Task {
			continue
		}
		receipt, readErr := readReleaseReceipt([]byte(result.Text))
		entry.Detail = result.Error
		now := time.Now().UTC()
		switch entry.State {
		case "building":
			entry.State, entry.Digest = "built", result.Text
			if result.Error != "" || entry.Digest == "" {
				entry.State = "failed"
			}
		case "reading-baseline":
			if readErr != nil || result.Error != "" {
				entry.State = "failed"
			} else if receipt.Digest != "" && !*receipt.Healthy {
				entry.State, entry.Detail = "failed", "The current live artifact is unhealthy. Restore a healthy version before promotion."
			} else if receipt.Digest == entry.Digest && *receipt.Healthy {
				entry.Previous, entry.Observed = receipt.Digest, receipt.Digest
				entry.State, entry.At = "deployed", &now
				entry.Detail = "Verified the artifact already serving this destination."
			} else {
				entry.Previous, entry.State = receipt.Digest, "staged"
			}
		case "deploying":
			entry.State = "verify"
		case "rolling-back":
			entry.State = "verify-rollback"
		case "verifying", "checking-rollback", "reconciling":
			rollback := entry.LastEffect == "rollback" || entry.State == "checking-rollback"
			entry.State = "uncertain"
			if readErr != nil || result.Error != "" {
				entry.Detail = "Live artifact could not be verified. Reconcile before another release."
				break
			}
			entry.Observed = receipt.Digest
			switch {
			case receipt.Digest == entry.Digest && *receipt.Healthy && !rollback:
				entry.State, entry.At = "deployed", &now
			case receipt.Digest == entry.Previous && (entry.Previous == "" || *receipt.Healthy):
				entry.State, entry.At = "failed", &now
				entry.Detail = "The previous artifact is active."
				if rollback {
					entry.State, entry.Detail = "rolled-back", "Verified the previous artifact after rollback."
				}
			case receipt.Digest == entry.Digest && !*receipt.Healthy && !rollback:
				entry.State, entry.Detail = "failed", "The promoted artifact failed its health check."
				if r.DeployConfig.Rollback && entry.Previous != "" {
					entry.State = "needs-rollback"
				}
			default:
				entry.Detail = "Destination reports an unexpected artifact or unhealthy rollback."
			}
		}
		return e.db.save(r, "release-"+entry.State)
	}
	return errors.New("release intent missing")
}

func (e *Engine) releaseCommand(r *Run, action Command) error {
	for i := range r.Releases {
		entry := &r.Releases[i]
		if entry.ID != action.Task {
			continue
		}
		switch action.Kind {
		case "hold-release", "resume-release":
			if !slices.Contains([]string{"queued", "building", "built", "reading-baseline", "staged"}, entry.State) {
				return errors.New("promotion already started or ended")
			}
			entry.Held = action.Kind == "hold-release"
		case "reconcile-release":
			if entry.State != "uncertain" {
				return errors.New("this release does not need reconciliation")
			}
			entry.State = "reconcile"
		}
		return nil
	}
	return fmt.Errorf("release %q is unavailable", action.Task)
}
