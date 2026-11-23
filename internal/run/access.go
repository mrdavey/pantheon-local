package run

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/project"
)

func (e *Engine) Log(runID, turnID string) ([]byte, error) {
	r, err := e.Get(runID)
	if err != nil {
		return nil, err
	}
	for _, turn := range r.Turns {
		if turn.ID == turnID {
			file, err := os.Open(filepath.Join(e.jobdir(r, turnID), "events.log"))
			if err != nil {
				return nil, err
			}
			defer file.Close()
			fi, err := file.Stat()
			if err != nil {
				return nil, err
			}
			const max = 256 * 1024
			if fi.Size() > max {
				if _, err = file.Seek(-max, io.SeekEnd); err != nil {
					return nil, err
				}
			}
			return io.ReadAll(io.LimitReader(file, max))
		}
	}
	return nil, errors.New("turn not found")
}

func (e *Engine) Preview(ctx context.Context, runID, candidateID string) (string, []string, error) {
	r, err := e.Get(runID)
	if err != nil {
		return "", nil, err
	}
	c := candidateByID(&r, candidateID)
	if c == nil || c.Commit == "" {
		return "", nil, errors.New("candidate has no preview source")
	}
	preview := c.Preview
	if c.Round == 0 {
		preview = r.Input.Recipe.Preview
	}
	if len(preview) == 0 {
		return "", nil, errors.New("this recipe has no preview command")
	}
	source := filepath.Join(e.root(r), "previews", uid())
	if err := project.Clone(ctx, e.integration(r), source, c.Commit, "swarm-preview"); err != nil {
		return "", nil, err
	}
	return workdir(source, r.Repo.Subdir), preview, nil
}

func (e *Engine) Advice(ctx context.Context, p adapter.Profile, prompt string) (string, error) {
	dir, err := os.MkdirTemp(e.dir, "advice-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(dir)
	env, err := workerEnvironment(e.dir)
	if err != nil {
		return "", err
	}
	result := adapter.Run(ctx, adapter.Request{Environment: env, Profile: p, Role: "advisor", Dir: dir, Prompt: fmt.Sprintf("Suggest a concise editable developer brief. Identify the highest-impact missing decision, if any. Preserve the developer's goal and distinguish assumptions from requirements. Respond in plain text.\n\n%s", prompt)}, func(adapter.Event) {})
	if result.Error != "" {
		return "", errors.New(result.Error)
	}
	return result.Text, nil
}

func (e *Engine) Diff(ctx context.Context, runID, candidateID string) (string, error) {
	r, err := e.Get(runID)
	if err != nil {
		return "", err
	}
	c := candidateByID(&r, candidateID)
	if c == nil || c.Commit == "" {
		return "", errors.New("candidate has no commit")
	}
	result, err := project.Diff(ctx, e.integration(r), c.Base, c.Commit)
	const displayLimit = 256 * 1024
	if len(result) > displayLimit {
		result = result[:displayLimit] + "\n[display truncated]\n"
	}
	return result, err
}
