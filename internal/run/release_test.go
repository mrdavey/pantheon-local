package run

import (
	"github.com/mrdavey/pantheon-local/internal/adapter"
	"testing"
)

func TestReleaseReadbackRequiresExplicitIdentityAndHealth(t *testing.T) {
	for _, raw := range []string{`{"healthy":true}`, `{"digest":null,"healthy":true}`, `{"digest":"sha256:old"}`} {
		if _, err := readReleaseReceipt([]byte(raw)); err == nil {
			t.Errorf("accepted incomplete readback %s", raw)
		}
	}
	receipt, err := readReleaseReceipt([]byte(`{"digest":"","healthy":false}`))
	if err != nil || receipt.Digest != "" || *receipt.Healthy {
		t.Fatalf("empty destination: %+v %v", receipt, err)
	}
}

func TestReconcileAlreadyLiveDigest(t *testing.T) {
	e, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	r := Run{ID: "run", Experiment: Experiment{Releases: []Release{{ID: "release", State: "reconciling", LastEffect: "deploy", Previous: "sha256:same", Digest: "sha256:same"}}}}
	if err := e.finishRelease(&r, Turn{Task: "release"}, adapter.Result{Text: `{"digest":"sha256:same","healthy":true}`}); err != nil {
		t.Fatal(err)
	}
	if got := r.Releases[0]; got.State != "deployed" || got.Observed != got.Digest || got.At == nil {
		t.Fatalf("wrong reconciliation: %+v", got)
	}
}
