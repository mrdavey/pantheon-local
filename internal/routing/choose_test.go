package routing

import (
	"net/http"
	"os"
	"testing"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
)

func price(cost float64) *float64 { return &cost }

func TestChooseWorker(t *testing.T) {
	profiles := map[string]adapter.Profile{
		"premium":   {Kind: "codex", Model: "gpt-pro", BenchmarkEffort: "max"},
		"efficient": {Kind: "claude", Model: "claude-regular"},
		"budget":    {Kind: "hermes", BenchmarkModel: "Cheap model"},
	}
	catalog := Catalog{Rows: []Evidence{
		{Model: "GPT Pro", Agent: "Codex", Effort: "max", Class: "terminal", Suite: "Test Bench", Date: "2026-10-02", Score: 91, Cost: price(1)},
		{Model: "GPT Pro", Agent: "Codex", Effort: "max", Class: "terminal", Suite: "Test Bench", Date: "2026-10-01", Score: 99, Cost: price(5)},
		{Model: "Claude Regular", Agent: "Claude Code", Class: "terminal", Suite: "Test Bench", Date: "2026-10-02", Score: 87, Cost: price(.25)},
		{Model: "Cheap model", Agent: "Hermes", Class: "terminal", Suite: "Test Bench", Date: "2026-10-02", Score: 76, Cost: price(.08)},
		{Model: "Cheap model", Agent: "Claude Code", Class: "terminal", Suite: "Test Bench", Date: "2026-10-03", Score: 99, Cost: price(.01)},
	}}
	for _, tc := range []struct {
		mode, class, want string
		evidence          bool
	}{
		{"manual", "testing", "premium", false},
		{"balanced", "testing", "efficient", true},
		{"quality", "debugging", "premium", true},
		{"economy", "implementation", "budget", true},
		{"balanced", "design", "premium", false},
	} {
		t.Run(tc.mode+"/"+tc.class, func(t *testing.T) {
			got, why := Choose(tc.mode, tc.class, "premium", []string{"premium", "efficient", "budget"}, profiles, catalog)
			if got != tc.want || (why != nil && why.Evidence != nil) != tc.evidence {
				t.Fatalf("worker=%q explanation=%+v", got, why)
			}
			if why != nil && why.Evidence != nil && (why.Compared != 3 || why.Evidence.Date != "2026-10-02") {
				t.Fatalf("unexpected evidence %+v", why)
			}
		})
	}
}

func TestAvoidCrossSuiteAndUnknownCostComparisons(t *testing.T) {
	profiles := map[string]adapter.Profile{"one": {Kind: "codex", Model: "gpt-test"}, "two": {Kind: "claude", Model: "opus-test"}}
	rows := []Evidence{
		{Model: "GPT Test", Agent: "Codex", Class: "testing", Suite: "Suite A", Source: "https://example.invalid/a", Score: 90, Cost: price(2)},
		{Model: "Opus Test", Agent: "Claude Code", Class: "testing", Suite: "Suite B", Source: "https://example.invalid/b", Score: 93, Cost: price(.1)},
	}
	workers := []string{"one", "two"}
	got, why := Choose("balanced", "testing", "one", workers, profiles, Catalog{Rows: rows})
	if got != "one" || why.Evidence != nil {
		t.Fatalf("%s %+v", got, why)
	}
	rows[1].Suite, rows[1].Source, rows[1].Cost = rows[0].Suite, rows[0].Source, nil
	got, why = Choose("balanced", "testing", "one", workers, profiles, Catalog{Rows: rows})
	if got != "one" || why.Evidence != nil {
		t.Fatalf("%s %+v", got, why)
	}
	got, why = Choose("quality", "testing", "one", workers, profiles, Catalog{Rows: rows})
	if got != "two" || why.Evidence == nil {
		t.Fatalf("%s %+v", got, why)
	}
}

func TestLiveTerminalBench(t *testing.T) {
	if os.Getenv("SWARM_TEST_LIVE_BENCHMARKS") != "1" {
		t.Skip("local credential-free opt-in")
	}
	got, err := fetchTerminal(t.Context(), &http.Client{Timeout: 10 * time.Second}, feedURL, time.Now().UTC())
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("Loaded %d official Terminal-Bench rows", len(got.Rows))
}

func TestOldRunWithUnusableScoresKeepsAssignedWorker(t *testing.T) {
	catalog := Catalog{}
	profiles := map[string]adapter.Profile{}
	for _, name := range []string{"one", "two"} {
		profiles[name] = adapter.Profile{Kind: "codex", Model: name}
		catalog.Rows = append(catalog.Rows, Evidence{Agent: "Codex", Model: name, Class: "terminal", Suite: suite, Source: sourceURL, Score: -1})
	}
	name, decision := Choose("quality", "testing", "two", []string{"one", "two"}, profiles, catalog)
	if name != "two" || decision == nil || decision.Evidence != nil {
		t.Fatalf("invalid fallback: %s %+v", name, decision)
	}
}
