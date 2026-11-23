package routing

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

const (
	feedURL   = "https://api.harborframework.com/functions/v1/api/v1/leaderboards/rows"
	sourceURL = "https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard"
	suite     = "Terminal-Bench 4.1"
)

type Evidence struct {
	Model  string   `json:"model"`
	Agent  string   `json:"agent"`
	Effort string   `json:"effort,omitempty"`
	Class  string   `json:"class"`
	Suite  string   `json:"suite"`
	Source string   `json:"source"`
	Date   string   `json:"date"`
	Score  float64  `json:"score"`
	Cost   *float64 `json:"cost_per_trial,omitempty"`
}

type Catalog struct {
	Fetched time.Time  `json:"fetched,omitempty"`
	Rows    []Evidence `json:"rows"`
	Note    string     `json:"note,omitempty"`
}

type harborPage struct {
	Next  string `json:"next_cursor"`
	Items []struct {
		Status   string `json:"status"`
		Trials   int    `json:"n_trials"`
		Metadata struct {
			Date   string                 `json:"date"`
			Agent  struct{ Label string } `json:"agent_display"`
			Model  struct{ Label string } `json:"model_display"`
			Effort string                 `json:"reasoning_effort"`
		}
		Metrics struct {
			Score *float64 `json:"accuracy"`
			Cost  *float64 `json:"total_cost_usd"`
		}
	} `json:"items"`
}

func fetchTerminal(ctx context.Context, client *http.Client, endpoint string, now time.Time) (Catalog, error) {
	q := url.Values{"package": {"terminal-bench/terminal-bench"}, "name": {"4-1-0"}, "page_size": {"100"}}
	out := Catalog{Fetched: now}
	for page := 0; page < 5; page++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint+"?"+q.Encode(), nil)
		if err != nil {
			return Catalog{}, err
		}
		req.Header.Set("Harbor-API-Version", "2026-09-10.preview")
		res, err := client.Do(req)
		if err != nil {
			return Catalog{}, err
		}
		var data harborPage
		err = json.NewDecoder(io.LimitReader(res.Body, 4<<20)).Decode(&data)
		_ = res.Body.Close()
		if res.StatusCode != http.StatusOK {
			return Catalog{}, fmt.Errorf("benchmark feed HTTP %d", res.StatusCode)
		}
		if err != nil {
			return Catalog{}, err
		}
		for _, row := range data.Items {
			if row.Status != "display" || row.Metrics.Score == nil {
				continue
			}
			var cost *float64
			if row.Trials > 0 && row.Metrics.Cost != nil && *row.Metrics.Cost > 0 {
				value := *row.Metrics.Cost / float64(row.Trials)
				cost = &value
			}
			e := Evidence{Model: row.Metadata.Model.Label, Agent: row.Metadata.Agent.Label, Effort: row.Metadata.Effort,
				Class: "terminal", Suite: suite, Source: sourceURL, Date: row.Metadata.Date, Score: *row.Metrics.Score, Cost: cost}
			if valid(e, now) {
				out.Rows = append(out.Rows, e)
			}
		}
		if data.Next == "" {
			if len(out.Rows) == 0 {
				return Catalog{}, errors.New("benchmark feed contained no usable rows")
			}
			return out, nil
		}
		q.Set("cursor", data.Next)
	}
	return Catalog{}, errors.New("benchmark feed exceeded page limit")
}

func Read(ctx context.Context, cachePath, localPath string, now time.Time) Catalog {
	cached, err := readCatalog(cachePath)
	if err != nil || cached.Fetched.After(now) {
		cached = Catalog{}
	}
	cached.Rows = slices.DeleteFunc(cached.Rows, func(row Evidence) bool { return !valid(row, now) })
	out := cached
	if len(cached.Rows) == 0 || now.Sub(cached.Fetched) >= 24*time.Hour {
		read, cancel := context.WithTimeout(ctx, 4*time.Second)
		client := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
		fresh, err := fetchTerminal(read, client, feedURL, now)
		cancel()
		if err == nil {
			out = fresh
			if raw, err := json.Marshal(fresh); err == nil {
				_ = writeCache(cachePath, raw)
			}
		} else if len(cached.Rows) > 0 && now.Sub(cached.Fetched) < 7*24*time.Hour {
			out.Note = "Live feed unavailable and recent cached results are in use."
		} else {
			out = Catalog{Note: "Live benchmark feed is unavailable."}
		}
	}
	if local, err := readCatalog(localPath); err == nil {
		for _, row := range local.Rows {
			if valid(row, now) {
				out.Rows = append(out.Rows, row)
			}
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		out.Note = strings.TrimSpace(out.Note + " Could not read local benchmark results.")
	}
	return out
}

func readCatalog(path string) (Catalog, error) {
	file, err := os.Open(path)
	if err != nil {
		return Catalog{}, err
	}
	defer file.Close()
	var data Catalog
	err = json.NewDecoder(io.LimitReader(file, 4<<20)).Decode(&data)
	return data, err
}

func writeCache(path string, raw []byte) error {
	tmp, err := os.CreateTemp(filepath.Dir(path), ".benchmark-")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	_, err = tmp.Write(raw)
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}

func valid(row Evidence, now time.Time) bool {
	date, err := time.Parse(time.DateOnly, row.Date)
	link, urlErr := url.Parse(row.Source)
	return row.Model != "" && row.Agent != "" && row.Suite != "" && row.Class != "" &&
		row.Score >= 0 && row.Score <= 100 && (row.Cost == nil || *row.Cost >= 0) &&
		err == nil && now.Sub(date) >= -24*time.Hour && now.Sub(date) < 180*24*time.Hour &&
		urlErr == nil && link.Scheme == "https" && link.Hostname() != "" && link.User == nil
}
