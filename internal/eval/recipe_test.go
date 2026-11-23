package eval

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestDarwinCandidateFilesystemBoundaries(t *testing.T) {
	if runtime.GOOS != "darwin" {
		t.Skip("macOS filesystem sandbox")
	}
	root := t.TempDir()
	source, data := filepath.Join(root, "source"), filepath.Join(root, "data")
	for _, dir := range []string{source, data} {
		if err := os.Mkdir(dir, 0700); err != nil {
			t.Fatal(err)
		}
	}
	control := filepath.Join(root, "token.json")
	if err := os.WriteFile(control, []byte("control"), 0600); err != nil {
		t.Fatal(err)
	}
	cmd := Command(t.Context(), source, data, []string{"/bin/sh", "-c",
		`echo ok > inside && echo ok > "$SWARM_DATA_DIR/inside" && ! /bin/cat ../token.json && ! /usr/bin/touch ../outside`}, Scope{State: root})
	if got, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("sandbox: %v %s", err, got)
	}
	for _, path := range []string{filepath.Join(source, "inside"), filepath.Join(data, "inside")} {
		if _, err := os.Stat(path); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := os.Stat(filepath.Join(root, "outside")); !os.IsNotExist(err) {
		t.Fatalf("outside write occurred: %v", err)
	}
}
