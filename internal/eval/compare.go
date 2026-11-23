package eval

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
)

type Case struct {
	ID          string          `json:"id"`
	Revision    int             `json:"revision,omitempty"`
	Driver      string          `json:"driver,omitempty"`
	Program     string          `json:"program,omitempty"`
	Interpreter string          `json:"interpreter,omitempty"`
	Digest      string          `json:"digest,omitempty"`
	Input       json.RawMessage `json:"input,omitempty"`
	Expected    json.RawMessage `json:"expected,omitempty"`
	Holdback    bool            `json:"holdback,omitempty"`
	Required    bool            `json:"required,omitempty"`
	Samples     int             `json:"samples,omitempty"`
	Seed        int64           `json:"seed,omitempty"`
}

type Metric struct {
	Name      string `json:"name"`
	Unit      string `json:"unit"`
	Direction string `json:"direction"`
	Aggregate string `json:"aggregate,omitempty"`
}

type Rule struct {
	Metrics    []Metric           `json:"metrics"`
	Objective  string             `json:"objective"`
	Minimum    float64            `json:"minimum,omitempty"`
	Limits     map[string]float64 `json:"limits,omitempty"`
	TieBreak   string             `json:"tie_break,omitempty"`
	MinSamples int                `json:"min_samples,omitempty"`
	MinLead    float64            `json:"min_lead,omitempty"`
}

type Summary struct {
	Value float64 `json:"value"`
	Min   float64 `json:"min"`
	Max   float64 `json:"max"`
	N     int     `json:"n"`
}

type Sample struct {
	Case     string             `json:"case"`
	Revision int                `json:"revision,omitempty"`
	Number   int                `json:"number"`
	Seed     int64              `json:"seed"`
	State    string             `json:"state"`
	Observed string             `json:"observed,omitempty"`
	Expected json.RawMessage    `json:"expected,omitempty"`
	Values   map[string]float64 `json:"values,omitempty"`
	Error    string             `json:"error,omitempty"`
}

type Report struct {
	Commit  string   `json:"commit"`
	Inputs  string   `json:"inputs"`
	Samples []Sample `json:"samples"`
	Checks  string   `json:"checks"`
}

func (r Recipe) validateComparison() error {
	cases := map[string]bool{}
	for _, c := range r.Cases {
		hasDriver := filepath.IsAbs(c.Driver) && c.Program == ""
		hasProgram := c.Driver == "" && c.Interpreter != "" && strings.TrimSpace(c.Program) != "" && len(c.Program) <= 4<<20
		if strings.TrimSpace(c.ID) == "" || cases[c.ID] || (!hasDriver && !hasProgram) || c.Samples < 0 || c.Samples > 100 || c.Revision < 0 {
			return errors.New("each eval case needs a unique ID, a driver path or interpreted program, and up to 100 samples")
		}
		cases[c.ID] = true
	}
	if r.Rule == nil {
		return nil
	}
	rule := r.Rule
	if len(r.Cases) == 0 || !finite(rule.Minimum) || rule.Minimum < 0 || !finite(rule.MinLead) || rule.MinLead < 0 || rule.MinSamples < 0 {
		return errors.New("selection rules need cases and nonnegative thresholds")
	}
	metrics := map[string]bool{}
	for _, m := range rule.Metrics {
		if m.Name == "" || m.Unit == "" || metrics[m.Name] || (m.Direction != "higher" && m.Direction != "lower") ||
			(m.Aggregate != "" && m.Aggregate != "mean" && m.Aggregate != "median" && m.Aggregate != "p95") {
			return errors.New("specify a unique metric name, unit, direction and supported aggregation")
		}
		metrics[m.Name] = true
	}
	if !metrics[rule.Objective] || (rule.TieBreak != "" && (!metrics[rule.TieBreak] || rule.TieBreak == rule.Objective)) {
		return errors.New("choose an objective and a distinct optional tie-break metric")
	}
	for name, threshold := range rule.Limits {
		if !metrics[name] || !finite(threshold) {
			return errors.New("limits must name a metric and finite threshold")
		}
	}
	return nil
}

func finite(v float64) bool { return !math.IsInf(v, 0) && !math.IsNaN(v) }

func fileDigest(raw []byte) string {
	digest := sha256.Sum256(raw)
	return hex.EncodeToString(digest[:])
}

// Pin copies developer-approved drivers into daemon-controlled storage.
func Pin(r Recipe, dir string) (Recipe, error) {
	if err := r.Validate(); err != nil {
		return r, err
	}
	r.Cases = slices.Clone(r.Cases)
	for i := range r.Cases {
		c := &r.Cases[i]
		raw := []byte(c.Program)
		if c.Driver != "" {
			info, err := os.Stat(c.Driver)
			if err != nil {
				return r, err
			}
			if !info.Mode().IsRegular() || info.Size() > 4<<20 {
				return r, errors.New("eval driver must be a regular file under 4 MiB")
			}
			raw, err = os.ReadFile(c.Driver)
			if err != nil {
				return r, err
			}
		}
		var err error
		c.Program = ""
		if c.Interpreter != "" {
			c.Interpreter, err = exec.LookPath(c.Interpreter)
			if err != nil {
				return r, err
			}
			if !filepath.IsAbs(c.Interpreter) {
				return r, errors.New("eval interpreter must resolve to an absolute path")
			}
		}
		c.Digest = fileDigest(raw)
		if err := os.MkdirAll(dir, 0700); err != nil {
			return r, err
		}
		c.Driver = filepath.Join(dir, c.Digest)
		if err := os.WriteFile(c.Driver, raw, 0500); err != nil && !errors.Is(err, os.ErrPermission) {
			return r, err
		}
		if err := verifyDriver(*c); err != nil {
			return r, err
		}
	}
	return r, nil
}

func verifyDriver(c Case) error {
	raw, err := os.ReadFile(c.Driver)
	if err != nil {
		return err
	}
	if c.Digest == "" || fileDigest(raw) != c.Digest {
		return errors.New("approved evaluator changed")
	}
	return nil
}

func Inputs(r Recipe, commit string) (string, error) {
	ruleFree := r
	ruleFree.Rule, ruleFree.Preview, ruleFree.Rubric = nil, nil, ""
	ruleFree.Cases = slices.Clone(r.Cases)
	binaries := map[string]string{}
	addBinary := func(program string) error {
		if _, ok := binaries[program]; ok {
			return nil
		}
		path, err := exec.LookPath(program)
		if err != nil || !filepath.IsAbs(path) {
			binaries[program] = "from candidate or unavailable"
			return nil
		}
		file, err := os.Open(path)
		if err != nil {
			return err
		}
		defer file.Close()
		hash := sha256.New()
		if _, err = io.Copy(hash, file); err != nil {
			return err
		}
		binaries[program] = path + ":" + hex.EncodeToString(hash.Sum(nil))
		return nil
	}
	for _, argv := range r.Checks {
		if err := addBinary(argv[0]); err != nil {
			return "", err
		}
		for _, arg := range argv[1:] {
			if filepath.IsAbs(arg) {
				info, err := os.Stat(arg)
				if err == nil && info.Mode().IsRegular() {
					raw, err := os.ReadFile(arg)
					if err != nil {
						return "", err
					}
					binaries[arg] = fileDigest(raw)
				}
			}
		}
	}
	for i, c := range r.Cases {
		if err := verifyDriver(c); err != nil {
			return "", err
		}
		if c.Interpreter != "" {
			if err := addBinary(c.Interpreter); err != nil {
				return "", err
			}
		}
		ruleFree.Cases[i].Driver = ""
	}
	payload, err := json.Marshal(struct {
		Recipe                               Recipe
		Commit, OS, Arch, Path, Lang, Locale string
		Binaries                             map[string]string
	}{ruleFree, commit, runtime.GOOS, runtime.GOARCH, os.Getenv("PATH"), os.Getenv("LANG"), os.Getenv("LC_ALL"), binaries})
	return fileDigest(payload), err
}

type limitedOutput struct {
	buffer bytes.Buffer
}

func (b *limitedOutput) Write(p []byte) (int, error) {
	if b.buffer.Len()+len(p) > 256<<10 {
		return 0, errors.New("evaluator output exceeds 256 KiB")
	}
	return b.buffer.Write(p)
}

func runCase(ctx context.Context, dir, data string, c Case, n int, scope Scope, log io.Writer) Sample {
	s := Sample{Case: c.ID, Revision: c.Revision, Number: n, Seed: c.Seed + int64(n-1), Expected: c.Expected, State: "error"}
	fail := func(err error) Sample { s.Error = err.Error(); return s }
	if err := verifyDriver(c); err != nil {
		return fail(err)
	}
	if err := os.MkdirAll(data, 0700); err != nil {
		return fail(err)
	}
	args := []string{c.Driver}
	if c.Interpreter != "" {
		args = append([]string{c.Interpreter}, args...)
	}
	// The immutable program is outside the candidate. The worker never receives
	// the case arguments. Evaluators should pass only the input to candidate code.
	cmd := command(ctx, dir, data, args, scope, c.Driver)
	input, err := json.Marshal(struct {
		Input    json.RawMessage `json:"input"`
		Expected json.RawMessage `json:"expected"`
		Sample   int             `json:"sample"`
		Seed     int64           `json:"seed"`
	}{c.Input, c.Expected, n, s.Seed})
	if err != nil {
		return fail(err)
	}
	cmd.Stdin = bytes.NewReader(input)
	var output limitedOutput
	cmd.Stdout, cmd.Stderr = &output, log
	if err := cmd.Run(); err != nil {
		return fail(err)
	}
	var result struct {
		Pass     *bool              `json:"pass"`
		Observed string             `json:"observed"`
		Metrics  map[string]float64 `json:"metrics"`
	}
	if err := json.Unmarshal(output.buffer.Bytes(), &result); err != nil {
		return fail(err)
	}
	if result.Pass == nil {
		return fail(errors.New("evaluator must emit a pass result"))
	}
	for _, value := range result.Metrics {
		if !finite(value) {
			return fail(errors.New("metric values must be finite"))
		}
	}
	s.State, s.Observed, s.Values = "pass", result.Observed, result.Metrics
	if !*result.Pass {
		s.State = "failed"
	}
	return s
}

func Measure(ctx context.Context, dir, data string, recipe Recipe, scope Scope, log io.Writer) ([]Sample, error) {
	var samples []Sample
	var failure error
	for i, c := range recipe.Cases {
		for n := 1; n <= max(1, c.Samples); n++ {
			s := runCase(ctx, dir, filepath.Join(data, fmt.Sprintf("case-%d-%d", i, n)), c, n, scope, log)
			samples = append(samples, s)
			fmt.Fprintf(log, "[case] %s #%d %s %s\n", c.ID, n, s.State, s.Error)
			if failure == nil && (s.State == "error" || (c.Required && s.State != "pass")) {
				failure = fmt.Errorf("eval case %s: %s %s", c.ID, s.State, s.Error)
			}
		}
	}
	return samples, failure
}

// Outcome validates a supervisor receipt against the approved sample schedule.
func (r Report) Outcome(recipe Recipe) (string, string) {
	if r.Checks != "pass" && r.Checks != "failed" {
		return "inconclusive", "Missing acceptance check result."
	}
	cases := make(map[string]Case, len(recipe.Cases))
	total := 0
	for _, c := range recipe.Cases {
		cases[c.ID] = c
		total += max(1, c.Samples)
	}
	if len(r.Samples) != total {
		return "inconclusive", fmt.Sprintf("Received %d of %d expected samples.", len(r.Samples), total)
	}
	type sampleID struct {
		id     string
		number int
	}
	seen := make(map[sampleID]bool, total)
	failure := ""
	if r.Checks == "failed" {
		failure = "An acceptance check failed."
	}
	for _, s := range r.Samples {
		c, ok := cases[s.Case]
		id := sampleID{s.Case, s.Number}
		if !ok || seen[id] || c.Revision != s.Revision || s.Number < 1 || s.Number > max(1, c.Samples) || s.Seed != c.Seed+int64(s.Number-1) {
			return "inconclusive", fmt.Sprintf("Unexpected sample for %s #%d.", s.Case, s.Number)
		}
		seen[id] = true
		switch s.State {
		case "pass":
		case "failed":
			if c.Required && failure == "" {
				failure = fmt.Sprintf("Required case %s failed on sample %d.", s.Case, s.Number)
			}
		default:
			return "inconclusive", fmt.Sprintf("No usable result for %s #%d.", s.Case, s.Number)
		}
	}
	if failure != "" {
		return "failed", failure
	}
	return "passed", ""
}

func Scores(samples []Sample, rule *Rule) map[string]Summary {
	if rule == nil {
		return nil
	}
	scores := map[string]Summary{}
	for _, m := range rule.Metrics {
		var values []float64
		for _, s := range samples {
			if v, ok := s.Values[m.Name]; ok {
				values = append(values, v)
			}
		}
		if len(values) < max(1, rule.MinSamples) {
			continue
		}
		slices.Sort(values)
		score := Summary{Min: values[0], Max: values[len(values)-1], N: len(values)}
		switch m.Aggregate {
		case "median":
			score.Value = values[len(values)/2]
			if len(values)%2 == 0 {
				score.Value = (score.Value + values[len(values)/2-1]) / 2
			}
		case "p95":
			score.Value = values[int(math.Ceil(0.95*float64(len(values))))-1]
		default:
			for _, v := range values {
				score.Value += v / float64(len(values))
			}
		}
		if finite(score.Value) {
			scores[m.Name] = score
		}
	}
	return scores
}
