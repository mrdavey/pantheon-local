package adapter

import "testing"

func TestCheckAvailability(t *testing.T) {
	for _, tc := range []struct {
		name, kind, script, state string
	}{
		{"codex signed in", "codex", "echo 'Logged in using ChatGPT'\n", "connected"},
		{"codex signed out", "codex", "echo 'Not logged in' >&2\nexit 1\n", "sign-in"},
		{"claude signed out", "claude", "echo '{\"loggedIn\":false}'\nexit 1\n", "sign-in"},
		{"claude signed in", "claude", "echo '{\"loggedIn\":true,\"email\":\"person@example.invalid\"}'\n", "connected"},
		{"unknown status", "claude", "echo '{}'\n", "sign-in-unknown"},
		{"Hermes installed", "hermes", "exit 42\n", "installed"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := Profile{Kind: tc.kind, Executable: fixture(t, tc.script)}
			if got := CheckAvailability(t.Context(), p); got.State != tc.state {
				t.Fatalf("got %q, want %q", got.State, tc.state)
			}
		})
	}
}
