package project

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func put(t *testing.T, dir, path, content string) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(dir, path), []byte(content), 0600); err != nil {
		t.Fatal(err)
	}
}

func gitMust(t *testing.T, dir string, args ...string) string {
	t.Helper()
	out, err := git(t.Context(), dir, args...)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func identity(t *testing.T, dir string) {
	t.Helper()
	gitMust(t, dir, "config", "user.name", "Test User")
	gitMust(t, dir, "config", "user.email", "swarm@example.invalid")
	gitMust(t, dir, "config", "commit.gpgsign", "false")
}

func TestNewAndExistingProjects(t *testing.T) {
	t.Setenv("GIT_CONFIG_GLOBAL", os.DevNull)
	ctx := context.Background()
	root := t.TempDir()
	gitMust(t, root, "init", "-b", "main")
	identity(t, root)
	repo, err := Open(ctx, root, nil, true)
	if err != nil || repo.Head == "" {
		t.Fatalf("empty project: %+v %v", repo, err)
	}
	sub := filepath.Join(root, "src")
	if err := os.Mkdir(sub, 0700); err != nil {
		t.Fatal(err)
	}
	put(t, root, "README.md", "mine\n")
	gitMust(t, root, "add", "README.md")
	put(t, root, "scratch.txt", "private\n")
	before := gitMust(t, root, "status", "--porcelain")
	found, err := Open(ctx, sub, nil, true)
	if err != nil || found.Head != repo.Head || found.Subdir != "src" {
		t.Fatalf("existing project: %+v %v", found, err)
	}
	worker := filepath.Join(t.TempDir(), "worker")
	if err := Clone(ctx, repo.Root, worker, repo.Head, "milestone"); err != nil {
		t.Fatal(err)
	}
	if err := ApplyCommitPolicy(ctx, repo.Root, worker); err != nil {
		t.Fatal(err)
	}
	parent := repo.Head
	for _, path := range []string{"api.txt", "ui.txt"} {
		put(t, worker, path, "implementation\n")
		gitMust(t, worker, "add", "--", path)
		gitMust(t, worker, "commit", "-m", "Implement "+path)
		sha, err := Milestone(ctx, worker, parent)
		if err != nil {
			t.Fatal(err)
		}
		parent = sha
	}
	if err := Import(ctx, root, worker, parent, "refs/heads/swarm-candidate"); err != nil {
		t.Fatal(err)
	}
	if err := os.RemoveAll(worker); err != nil {
		t.Fatal(err)
	}
	if err := Land(ctx, root, repo.Target, repo.Head, parent); err == nil {
		t.Fatal("dirty checkout landed")
	}
	if got := gitMust(t, root, "status", "--porcelain"); got != before {
		t.Fatalf("developer work changed: %s", got)
	}
	gitMust(t, root, "switch", "--detach", repo.Head)
	detached, err := Open(ctx, root, nil, true)
	if err != nil || detached.Target != "" {
		t.Fatalf("detached: %+v %v", detached, err)
	}
	if err := Land(ctx, root, repo.Target, repo.Head, parent); err != nil {
		t.Fatal(err)
	}
	if got := gitMust(t, root, "diff", "--name-only", repo.Head, parent); got != "api.txt\nui.txt" {
		t.Fatalf("changes: %s", got)
	}
}

func TestInitialImportAndLinkedWorktree(t *testing.T) {
	root := t.TempDir()
	gitMust(t, root, "init", "-b", "main")
	identity(t, root)
	put(t, root, ".gitignore", "secret\n")
	put(t, root, "secret", "private\n")
	put(t, root, "project.txt", "source\n")
	if _, err := Open(t.Context(), root, nil, true); err == nil || !strings.Contains(err.Error(), "initial files explicitly") {
		t.Fatalf("expected explicit import: %v", err)
	}
	if _, err := Open(t.Context(), root, []string{"secret"}, true); err == nil {
		t.Fatal("ignored file admitted")
	}
	repo, err := Open(t.Context(), root, []string{".gitignore", "project.txt"}, true)
	if err != nil {
		t.Fatal(err)
	}
	if status := gitMust(t, root, "status", "--porcelain"); status != "" {
		t.Fatalf("unexpected status %q", status)
	}
	other := filepath.Join(t.TempDir(), "linked")
	gitMust(t, root, "worktree", "add", "-b", "other", other)
	got, err := Discover(t.Context(), other)
	if err != nil || got.Common != repo.Common || got.Target != "refs/heads/other" {
		t.Fatalf("linked: %+v %v", got, err)
	}
	if err := Land(t.Context(), other, got.Target, "stale", repo.Head+"0"); err == nil || !strings.Contains(err.Error(), "target moved") {
		t.Fatalf("expected stale error: %v", err)
	}
}

func TestInitialImportKeepsSelectedSubdirectory(t *testing.T) {
	root := t.TempDir()
	gitMust(t, root, "init", "-b", "main")
	identity(t, root)
	sub := filepath.Join(root, "app")
	if err := os.Mkdir(sub, 0700); err != nil {
		t.Fatal(err)
	}
	put(t, sub, "hello.txt", "hello")
	repo, err := Open(t.Context(), sub, []string{"app/hello.txt"}, true)
	if err != nil || repo.Subdir != "app" {
		t.Fatalf("initial import from app: %+v %v", repo, err)
	}
}

func TestRepairChangesPreservesCleanMergeResults(t *testing.T) {
	ctx := t.Context()
	source := t.TempDir()
	gitMust(t, source, "init", "-b", "main")
	identity(t, source)
	put(t, source, "overlap", "base")
	gitMust(t, source, "add", ".")
	gitMust(t, source, "commit", "-m", "Base")
	gitMust(t, source, "switch", "-c", "right")
	put(t, source, "overlap", "right")
	put(t, source, "clean", "right only")
	gitMust(t, source, "add", ".")
	gitMust(t, source, "commit", "-m", "Right")
	gitMust(t, source, "switch", "main")
	put(t, source, "overlap", "left")
	gitMust(t, source, "commit", "-am", "Left")
	if Merge(ctx, source, "right") == nil {
		t.Fatal("expected conflict")
	}
	worker := filepath.Join(t.TempDir(), "repair")
	if err := Clone(ctx, source, worker, gitMust(t, source, "rev-parse", "HEAD"), "repair"); err != nil {
		t.Fatal(err)
	}
	identity(t, worker)
	right := gitMust(t, source, "rev-parse", "right")
	if err := Merge(ctx, worker, right); err == nil {
		t.Fatal("expected worker conflict")
	}
	put(t, worker, "overlap", "both")
	gitMust(t, worker, "commit", "-am", "Resolve overlap")
	check := func(want string) {
		t.Helper()
		got, err := RepairChanges(ctx, source, worker, gitMust(t, worker, "rev-parse", "HEAD"))
		if err != nil || strings.Join(got, ",") != want {
			t.Fatalf("repair paths = %q, want %q: %v", got, want, err)
		}
	}
	check("overlap")
	put(t, worker, "clean", "unrequested edit")
	gitMust(t, worker, "commit", "-am", "Alter another file")
	check("clean,overlap")
}
