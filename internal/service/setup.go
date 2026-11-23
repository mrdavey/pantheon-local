package service

import (
	"context"
	"errors"
	"net/http"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
)

func (s *Server) setupStatus(w http.ResponseWriter, r *http.Request) {
	setup, err := ReadSetup(s.dir)
	if err != nil {
		result(w, nil, err)
		return
	}
	_, gitErr := exec.LookPath("git")
	report := struct {
		Setup    Setup                           `json:"setup"`
		Git      bool                            `json:"git"`
		Profiles map[string]adapter.Availability `json:"profiles"`
	}{Setup: setup, Git: gitErr == nil, Profiles: make(map[string]adapter.Availability, len(setup.Profiles))}
	ctx, cancel := context.WithTimeout(r.Context(), 10*time.Second)
	defer cancel()
	for name, profile := range setup.Profiles {
		probe, done := context.WithTimeout(ctx, 2*time.Second)
		report.Profiles[name] = adapter.CheckAvailability(probe, profile)
		done()
	}
	result(w, report, nil)
}

func (s *Server) setupModel(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Profile         string `json:"profile"`
		Model           string `json:"model"`
		BenchmarkModel  string `json:"benchmark_model"`
		BenchmarkAgent  string `json:"benchmark_agent"`
		BenchmarkEffort string `json:"benchmark_effort"`
	}
	if err := decode(w, r, &input); err != nil {
		result(w, nil, err)
		return
	}
	for _, value := range []string{input.Model, input.BenchmarkModel, input.BenchmarkAgent, input.BenchmarkEffort} {
		if len(value) > 150 || strings.ContainsAny(value, "\r\n\x00") {
			result(w, nil, errors.New("model settings must be one line and at most 150 bytes"))
			return
		}
	}
	s.profileMu.Lock()
	defer s.profileMu.Unlock()
	setup, err := ReadSetup(s.dir)
	if err != nil {
		result(w, nil, err)
		return
	}
	profile, ok := setup.Profiles[input.Profile]
	if !ok {
		result(w, nil, errors.New("agent profile not found"))
		return
	}
	if profile.Kind == "codex" || profile.Kind == "claude" {
		profile.Model = strings.TrimSpace(input.Model)
	}
	profile.BenchmarkModel = strings.TrimSpace(input.BenchmarkModel)
	profile.BenchmarkAgent = strings.TrimSpace(input.BenchmarkAgent)
	profile.BenchmarkEffort = strings.TrimSpace(input.BenchmarkEffort)
	setup.Profiles[input.Profile] = profile
	err = writeLocal(filepath.Join(s.dir, "profiles.json"), setup)
	result(w, setup, err)
}
