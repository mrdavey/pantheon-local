package routing

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestTerminalFeedPaginationAndCosts(t *testing.T) {
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Harbor-API-Version") == "" || r.URL.Query().Get("name") != "4-1-0" {
			t.Error("missing feed contract")
		}
		model, next, date := "Model A", "another", "2026-11-20"
		if r.URL.Query().Get("cursor") == "another" {
			model, next, date = "Model B", "", "2026-11-22"
		}
		fmt.Fprintf(w, `{"next_cursor":%q,"items":[
		  {"status":"display","n_trials":5,"metadata":{"date":%q,"agent_display":{"label":"Codex"},"model_display":{"label":%q}},"metrics":{"accuracy":88.4,"total_cost_usd":2.5}},
		  {"status":"hide","metadata":{"date":%q,"agent_display":{"label":"Codex"},"model_display":{"label":"hidden"}},"metrics":{"accuracy":100}},
		  {"status":"display","metadata":{"date":"2025-11-20","agent_display":{"label":"Codex"},"model_display":{"label":"old"}},"metrics":{"accuracy":100}}
		]}`, next, date, model, date)
	})
	server := httptest.NewServer(handler)
	defer server.Close()
	got, err := fetchTerminal(t.Context(), server.Client(), server.URL, time.Date(2026, 11, 22, 12, 0, 0, 0, time.UTC))
	if err != nil || len(got.Rows) != 2 {
		t.Fatalf("%+v %v", got, err)
	}
	for _, row := range got.Rows {
		if row.Score != 88.4 || row.Cost == nil || *row.Cost != .5 || row.Source != sourceURL {
			t.Fatalf("%+v", row)
		}
	}
}

func TestCachedBenchmarkEvidenceMustStillBeUsable(t *testing.T) {
	now := time.Date(2026, 11, 23, 12, 0, 0, 0, time.UTC)
	row := Evidence{Agent: "Codex", Model: "Model", Class: "terminal", Suite: suite, Source: sourceURL, Date: "2026-11-22", Score: 80}
	expired := row
	expired.Date = "2025-11-22"
	bad := row
	bad.Score = -5
	dir := t.TempDir()
	path := filepath.Join(dir, "cache")
	store := func(fetched time.Time, rows ...Evidence) {
		raw, err := json.Marshal(Catalog{Fetched: fetched, Rows: rows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, raw, 0600); err != nil {
			t.Fatal(err)
		}
	}
	store(now, row, expired, bad)
	got := Read(t.Context(), path, filepath.Join(dir, "local"), now)
	if len(got.Rows) != 1 || got.Rows[0].Date != row.Date {
		t.Fatalf("cache supplied invalid rows: %+v", got.Rows)
	}
	store(now.Add(time.Hour), row)
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	got = Read(ctx, path, filepath.Join(dir, "local"), now)
	if len(got.Rows) > 0 {
		t.Fatalf("future cache accepted on failed refresh: %+v", got)
	}
}
