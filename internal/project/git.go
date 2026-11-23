package project

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Repository struct {
	Root   string
	Common string
	Subdir string
	Head   string
	Target string
}

func git(ctx context.Context, dir string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	cmd.Env = gitEnv()
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %s (%w)", args[0], strings.TrimSpace(string(out)), err)
	}
	return strings.TrimSuffix(string(out), "\n"), nil
}

func gitEnv() []string {
	var env []string
	for _, value := range os.Environ() {
		if !strings.HasPrefix(value, "GIT_") {
			env = append(env, value)
		}
	}
	return env
}

func Discover(ctx context.Context, path string) (Repository, error) {
	path, err := filepath.Abs(path)
	if err != nil {
		return Repository{}, err
	}
	path, err = filepath.EvalSymlinks(path)
	if err != nil {
		return Repository{}, err
	}
	root, err := git(ctx, path, "rev-parse", "--show-toplevel")
	if err != nil {
		return Repository{}, err
	}
	common, err := git(ctx, root, "rev-parse", "--path-format=absolute", "--git-common-dir")
	if err != nil {
		return Repository{}, err
	}
	subdir, err := filepath.Rel(root, path)
	if err != nil {
		return Repository{}, err
	}
	target, _ := git(ctx, root, "symbolic-ref", "--quiet", "HEAD")
	head, err := git(ctx, root, "rev-parse", "--verify", "HEAD^{commit}")
	if err != nil && target == "" {
		return Repository{}, err
	}
	if err != nil && !strings.Contains(err.Error(), "Needed a single revision") {
		return Repository{}, err
	}
	if err == nil && head == "" {
		return Repository{}, errors.New("empty revision")
	}
	return Repository{Root: root, Common: common, Subdir: subdir, Head: head, Target: target}, nil
}

func Open(ctx context.Context, path string, initialFiles []string, initialize bool) (Repository, error) {
	repo, err := Discover(ctx, path)
	if err != nil {
		if !initialize || !strings.Contains(err.Error(), "not a git repository") {
			return Repository{}, err
		}
		canonical, resolveErr := filepath.EvalSymlinks(path)
		if resolveErr != nil {
			return Repository{}, resolveErr
		}
		canonical, resolveErr = filepath.Abs(canonical)
		if resolveErr != nil {
			return Repository{}, resolveErr
		}
		for dir := canonical; ; dir = filepath.Dir(dir) {
			if _, statErr := os.Lstat(filepath.Join(dir, ".git")); !errors.Is(statErr, os.ErrNotExist) {
				return Repository{}, fmt.Errorf("cannot open enclosing Git metadata in %s: %w", dir, err)
			}
			if filepath.Dir(dir) == dir {
				break
			}
		}
		if _, err := git(ctx, canonical, "init", "--quiet"); err != nil {
			return Repository{}, err
		}
		repo, err = Discover(ctx, canonical)
		if err != nil {
			return Repository{}, err
		}
	}
	if repo.Head != "" {
		if len(initialFiles) > 0 {
			return Repository{}, errors.New("initial files are only accepted on an unborn branch")
		}
		return repo, nil
	}
	if !initialize {
		return Repository{}, errors.New("initialize or commit a base before starting")
	}
	staged, err := git(ctx, repo.Root, "ls-files", "-z")
	if err != nil {
		return Repository{}, err
	}
	if staged != "" {
		return Repository{}, errors.New("create the initial commit in Git to preserve your staged work")
	}
	untracked, err := git(ctx, repo.Root, "ls-files", "--others", "--exclude-standard", "-z")
	if err != nil {
		return Repository{}, err
	}
	if untracked != "" && len(initialFiles) == 0 {
		return Repository{}, fmt.Errorf("choose initial files explicitly (or commit in Git): %q", strings.Split(strings.TrimSuffix(untracked, "\x00"), "\x00"))
	}
	// An alternate index keeps a failed stage or hook away from the user's index.
	scratch, err := os.MkdirTemp("", "swarm-index-")
	if err != nil {
		return Repository{}, err
	}
	defer os.RemoveAll(scratch)
	run := func(args ...string) error {
		cmd := exec.CommandContext(ctx, "git", args...)
		cmd.Dir = repo.Root
		cmd.Env = append(gitEnv(), "GIT_INDEX_FILE="+filepath.Join(scratch, "index"))
		out, err := cmd.CombinedOutput()
		if err != nil {
			return fmt.Errorf("git %s: %s (%w)", args[0], strings.TrimSpace(string(out)), err)
		}
		return nil
	}
	for _, name := range initialFiles {
		full := filepath.Join(repo.Root, name)
		info, statErr := os.Lstat(full)
		if !filepath.IsLocal(name) || statErr != nil || (!info.Mode().IsRegular() && info.Mode()&os.ModeSymlink == 0) {
			return Repository{}, fmt.Errorf("initial import requires a repo-relative file: %q", name)
		}
	}
	if len(initialFiles) > 0 {
		if err := run(append([]string{"--literal-pathspecs", "add", "--"}, initialFiles...)...); err != nil {
			return Repository{}, err
		}
	}
	if err := run("commit", "--quiet", "--allow-empty", "-m", "Establish the project base"); err != nil {
		return Repository{}, err
	}
	if len(initialFiles) > 0 {
		// Populate entries after the alternate index has committed the selected files.
		if _, err := git(ctx, repo.Root, append([]string{"--literal-pathspecs", "reset", "--quiet", "HEAD", "--"}, initialFiles...)...); err != nil {
			return Repository{}, err
		}
	}
	return Discover(ctx, path)
}

func Clone(ctx context.Context, source, dir, base, branch string) error {
	if err := os.MkdirAll(filepath.Dir(dir), 0700); err != nil {
		return err
	}
	if _, err := git(ctx, "", "clone", "--no-local", "--no-checkout", "--quiet", "--", source, dir); err != nil {
		return err
	}
	if _, err := git(ctx, dir, "fetch", "--no-tags", "--quiet", "--", source, base); err != nil {
		return err
	}
	if _, err := git(ctx, dir, "checkout", "--quiet", "-B", branch, base); err != nil {
		return err
	}
	_, err := git(ctx, dir, "remote", "remove", "origin")
	return err
}

func ApplyCommitPolicy(ctx context.Context, source, target string) error {
	for _, key := range []string{"user.name", "user.email", "user.signingkey", "commit.gpgsign", "gpg.format", "gpg.program", "gpg.ssh.program"} {
		if value, err := git(ctx, source, "config", "--get", key); err == nil {
			if _, err := git(ctx, target, "config", "--local", key, value); err != nil {
				return err
			}
		}
	}
	hooks, err := git(ctx, source, "rev-parse", "--path-format=absolute", "--git-path", "hooks")
	if err != nil {
		return err
	}
	_, err = git(ctx, target, "config", "--local", "core.hooksPath", hooks)
	return err
}

func Head(ctx context.Context, dir string) (string, error) {
	return git(ctx, dir, "rev-parse", "--verify", "HEAD^{commit}")
}

func Status(ctx context.Context, dir string) (string, error) {
	return git(ctx, dir, "--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all")
}

func IsAncestor(ctx context.Context, dir, base, sha string) bool {
	_, err := git(ctx, dir, "merge-base", "--is-ancestor", base, sha)
	return err == nil
}

func Milestone(ctx context.Context, dir, base string) (string, error) {
	status, err := Status(ctx, dir)
	if err != nil {
		return "", err
	}
	if status != "" {
		return "", errors.New("milestone contains uncommitted files")
	}
	return ReportedMilestone(ctx, dir, base)
}

func ReportedMilestone(ctx context.Context, dir, base string) (string, error) {
	head, err := Head(ctx, dir)
	if err != nil {
		return "", err
	}
	if head == base || !IsAncestor(ctx, dir, base, head) {
		return "", errors.New("milestone must commit a descendant of its assigned base")
	}
	delta, err := git(ctx, dir, "diff", "--name-only", "-z", base, head)
	if err != nil {
		return "", err
	}
	if delta == "" {
		return "", errors.New("milestone has no changed files")
	}
	return head, nil
}

func Import(ctx context.Context, target, source, sha, ref string) error {
	if _, err := git(ctx, target, "fetch", "--no-tags", "--quiet", "--", source, sha); err != nil {
		return err
	}
	_, err := git(ctx, target, "update-ref", ref, sha, strings.Repeat("0", len(sha)))
	if err != nil {
		current, readErr := git(ctx, target, "rev-parse", "--verify", ref)
		if readErr == nil && current == sha {
			return nil
		}
	}
	return err
}

func Merge(ctx context.Context, dir, sha string) error {
	_, err := git(ctx, dir, "-c", "core.hooksPath="+os.DevNull, "merge", "--no-edit", sha)
	return err
}

func Diff(ctx context.Context, dir, base, sha string) (string, error) {
	return git(ctx, dir, "diff", "--no-ext-diff", base, sha, "--")
}

func TargetHead(ctx context.Context, repo, ref string) (string, error) {
	if !strings.HasPrefix(ref, "refs/heads/") {
		return "", errors.New("choose a local branch")
	}
	return git(ctx, repo, "show-ref", "--verify", "--hash", ref)
}

func Land(ctx context.Context, repo, ref, base, candidate string) error {
	head, err := TargetHead(ctx, repo, ref)
	if err != nil {
		return err
	}
	if head == candidate {
		return nil
	}
	if head != base {
		return fmt.Errorf("target moved from %s to %s", base, head)
	}
	if !IsAncestor(ctx, repo, base, candidate) {
		return errors.New("candidate cannot fast-forward target")
	}
	worktrees, err := git(ctx, repo, "worktree", "list", "--porcelain", "-z")
	if err != nil {
		return err
	}
	var checkout string
	for _, field := range bytes.Split([]byte(worktrees), []byte{0}) {
		value := string(field)
		if path, ok := strings.CutPrefix(value, "worktree "); ok {
			checkout = path
		}
		if value != "branch "+ref {
			continue
		}
		if _, err := os.Stat(checkout); err != nil {
			return err
		}
		for _, name := range []string{"MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "sequencer", "BISECT_START"} {
			path, err := git(ctx, checkout, "rev-parse", "--path-format=absolute", "--git-path", name)
			if err != nil {
				return err
			}
			if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
				return errors.New("target checkout has a Git operation in progress")
			}
		}
		dirty, err := Status(ctx, checkout)
		if err != nil {
			return err
		}
		if dirty != "" {
			return errors.New("target checkout has local changes")
		}
		checked, err := git(ctx, checkout, "symbolic-ref", "--quiet", "HEAD")
		if err != nil || checked != ref {
			return errors.New("target checkout changed branches")
		}
		checkedHead, err := Head(ctx, checkout)
		if err != nil || checkedHead != base {
			return errors.New("target checkout moved")
		}
		if _, err = git(ctx, checkout, "merge", "--ff-only", "--no-overwrite-ignore", candidate); err != nil {
			return err
		}
		after, err := TargetHead(ctx, repo, ref)
		if err != nil {
			return err
		}
		if after != candidate {
			return errors.New("target changed during landing; inspect its current revision")
		}
		return nil
	}
	_, err = git(ctx, repo, "update-ref", ref, candidate, base)
	return err
}

func Files(ctx context.Context, dir, base string) (string, error) {
	return git(ctx, dir, "ls-tree", "-r", "--name-only", base)
}

func Fetch(ctx context.Context, dir, source, sha string) error {
	_, err := git(ctx, dir, "fetch", "--quiet", "--no-tags", "--", source, sha)
	return err
}

func ReadBlob(ctx context.Context, dir, base, path string) (string, error) {
	return git(ctx, dir, "show", base+":"+filepath.ToSlash(path))
}

func CheckSnapshot(ctx context.Context, dir, expected string) error {
	head, err := Head(ctx, dir)
	if err != nil {
		return err
	}
	if head != expected {
		return errors.New("checkout commit does not match the candidate")
	}
	status, err := git(ctx, dir, "--no-optional-locks", "status", "--porcelain=v1", "--untracked-files=no", "--ignore-submodules=none")
	if err != nil {
		return err
	}
	if status != "" {
		return errors.New("candidate has modifications to tracked files")
	}
	return nil
}

func ChangedPaths(ctx context.Context, dir, base, sha string) ([]string, error) {
	paths, err := git(ctx, dir, "diff", "--no-renames", "--name-only", "-z", base, sha, "--")
	if err != nil {
		return nil, err
	}
	return strings.Split(strings.TrimSuffix(paths, "\x00"), "\x00"), nil
}

// RepairChanges compares the resolved commit with Git's staged merge result.
// The private integration index still contains every cleanly merged file.
func RepairChanges(ctx context.Context, unresolved, worker, commit string) ([]string, error) {
	if err := Fetch(ctx, unresolved, worker, commit); err != nil {
		return nil, err
	}
	out, err := git(ctx, unresolved, "diff", "--cached", "--no-renames", "--name-only", "-z", commit, "--")
	if err != nil || out == "" {
		return nil, err
	}
	return strings.Split(strings.TrimSuffix(out, "\x00"), "\x00"), nil
}

func ConflictedPaths(ctx context.Context, dir string) ([]string, error) {
	out, err := git(ctx, dir, "diff", "--name-only", "--diff-filter=U", "-z")
	if err != nil || out == "" {
		return nil, err
	}
	return strings.Split(strings.TrimSuffix(out, "\x00"), "\x00"), nil
}

func Pin(ctx context.Context, dir, ref, sha string) error {
	_, err := git(ctx, dir, "update-ref", ref, sha)
	return err
}
