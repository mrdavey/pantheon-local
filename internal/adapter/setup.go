package adapter

import (
	"context"
	"encoding/json"
	"os/exec"
	"strings"
)

type Availability struct {
	State string `json:"state"`
}

// CheckAvailability uses native local status commands and returns no account data.
func CheckAvailability(ctx context.Context, profile Profile) Availability {
	if profile.Validate() != nil {
		return Availability{State: "invalid"}
	}
	if _, err := exec.LookPath(profile.Binary()); err != nil {
		return Availability{State: "missing"}
	}
	switch profile.Kind {
	case "codex":
		raw, err := exec.CommandContext(ctx, profile.Binary(), "login", "status").CombinedOutput()
		status := strings.TrimSpace(strings.ToLower(string(raw)))
		if strings.HasPrefix(status, "not logged in") {
			return Availability{State: "sign-in"}
		}
		if err == nil && strings.HasPrefix(status, "logged in") {
			return Availability{State: "connected"}
		}
	case "claude":
		raw, _ := exec.CommandContext(ctx, profile.Binary(), "auth", "status", "--json").Output()
		var status struct {
			LoggedIn *bool `json:"loggedIn"`
		}
		if json.Unmarshal(raw, &status) == nil && status.LoggedIn != nil {
			if *status.LoggedIn {
				return Availability{State: "connected"}
			}
			return Availability{State: "sign-in"}
		}
	}
	if profile.Kind == "codex" || profile.Kind == "claude" {
		return Availability{State: "sign-in-unknown"}
	}
	return Availability{State: "installed"}
}
