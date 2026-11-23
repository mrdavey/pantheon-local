package routing

import (
	"slices"
	"strings"
	"unicode"

	"github.com/mrdavey/pantheon-local/internal/adapter"
)

type Decision struct {
	Mode     string    `json:"mode"`
	Reason   string    `json:"reason"`
	Evidence *Evidence `json:"evidence,omitempty"`
	Compared int       `json:"compared,omitempty"`
}

func ModeValid(mode string) bool {
	return slices.Contains([]string{"", "manual", "balanced", "quality", "economy"}, mode)
}

func ClassValid(class string) bool {
	return slices.Contains([]string{"", "implementation", "debugging", "testing", "automation", "documentation", "design"}, class)
}

func comparable(class, evidence string) bool {
	if class == evidence {
		return true
	}
	return evidence == "terminal" && slices.Contains([]string{"implementation", "debugging", "testing", "automation"}, class)
}

func norm(value string) string {
	return strings.Map(func(r rune) rune {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			return unicode.ToLower(r)
		}
		return -1
	}, value)
}

func matches(p adapter.Profile, e Evidence) bool {
	model := p.Model
	if p.BenchmarkModel != "" {
		model = p.BenchmarkModel
	}
	if model == "" || norm(model) != norm(e.Model) {
		return false
	}
	if p.BenchmarkEffort != "" && norm(p.BenchmarkEffort) != norm(e.Effort) {
		return false
	}
	if p.BenchmarkAgent != "" {
		return norm(p.BenchmarkAgent) == norm(e.Agent)
	}
	switch p.Kind {
	case "codex":
		return norm(e.Agent) == "codex" || norm(e.Agent) == "codexcli"
	case "claude":
		return norm(e.Agent) == "claudecode"
	case "hermes":
		return norm(e.Agent) == "hermes" || norm(e.Agent) == "hermesagent"
	case "antigravity":
		return norm(e.Agent) == "antigravity" || norm(e.Agent) == "antigravitycli"
	}
	return false
}

type option struct {
	name string
	e    Evidence
}

func Choose(mode, class, fallback string, workers []string, profiles map[string]adapter.Profile, catalog Catalog) (string, *Decision) {
	if mode == "" || mode == "manual" {
		return fallback, nil
	}
	failed := func(reason string) (string, *Decision) { return fallback, &Decision{Mode: mode, Reason: reason} }
	if class == "" {
		return failed("The coordinator did not specify a benchmark category.")
	}
	var suiteOptions = map[string]map[string]option{}
	for _, row := range catalog.Rows {
		if !comparable(class, row.Class) {
			continue
		}
		if mode != "quality" && row.Cost == nil {
			continue
		}
		key := row.Suite + "\x00" + row.Source + "\x00" + row.Class
		if suiteOptions[key] == nil {
			suiteOptions[key] = map[string]option{}
		}
		for _, name := range workers {
			p := profiles[name]
			if !matches(p, row) {
				continue
			}
			e := row
			if old, ok := suiteOptions[key][name]; !ok || e.Date > old.e.Date || (e.Date == old.e.Date && e.Score > old.e.Score) {
				suiteOptions[key][name] = option{name, e}
			}
		}
	}
	var chosen []option
	var chosenKey, newest string
	var exact bool
	for key, options := range suiteOptions {
		if len(options) < 2 {
			continue
		}
		var group []option
		var latest string
		for _, o := range options {
			group = append(group, o)
			latest = max(latest, o.e.Date)
		}
		direct := group[0].e.Class == class
		if chosen == nil || (direct && !exact) || (direct == exact && (latest > newest || (latest == newest && key < chosenKey))) {
			chosen, chosenKey, newest, exact = group, key, latest, direct
		}
	}
	if len(chosen) < 2 {
		reason := "Fewer than two selected profiles have comparable evidence."
		if catalog.Note != "" {
			reason += " " + catalog.Note
		}
		return failed(reason)
	}
	best := 0.0
	for _, o := range chosen {
		best = max(best, o.e.Score)
	}
	margin := 0.0
	switch mode {
	case "balanced":
		margin = 5
	case "economy":
		margin = 15
	}
	var winner *option
	for _, o := range chosen {
		if o.e.Score < best-margin {
			continue
		}
		if winner == nil || prefer(o, *winner, mode) {
			winner = &o
		}
	}
	if winner == nil {
		return failed("The saved snapshot has no usable scores for these profiles.")
	}
	reason := "Highest measured score."
	if mode == "balanced" {
		reason = "Lowest measured cost within five points of the best score."
	}
	if mode == "economy" {
		reason = "Lowest measured cost within fifteen points of the best score."
	}
	return winner.name, &Decision{Mode: mode, Reason: reason, Evidence: &winner.e, Compared: len(chosen)}
}

func prefer(a, b option, mode string) bool {
	if mode == "quality" && a.e.Score != b.e.Score {
		return a.e.Score > b.e.Score
	}
	if a.e.Cost != nil && b.e.Cost == nil {
		return true
	}
	if a.e.Cost == nil && b.e.Cost != nil {
		return false
	}
	if a.e.Cost != nil && b.e.Cost != nil && *a.e.Cost != *b.e.Cost {
		return *a.e.Cost < *b.e.Cost
	}
	if a.e.Score != b.e.Score {
		return a.e.Score > b.e.Score
	}
	return a.name < b.name
}
