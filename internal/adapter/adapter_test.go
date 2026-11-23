package adapter

import (
	"os"
	"path/filepath"
	"testing"
)

func fixture(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "agent")
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"+body), 0700); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestNativeCodex(t *testing.T) {
	bin := fixture(t, `test "$1 $2 $3 $4 $5" = "exec resume --json abc -" || exit 8
read -r prompt
test "$prompt" = hello || exit 9
cat <<'JSON'
{"type":"thread.started","thread_id":"abc"}
{"type":"item.completed","item":{"type":"agent_message","text":"done"}}
{"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":2}}
JSON
`)
	got := Run(t.Context(), Request{Profile: Profile{Kind: "codex", Executable: bin}, Dir: t.TempDir(), Session: "abc", Prompt: "hello\n"}, func(Event) {})
	if got.Text != "done" || got.Session != "abc" || got.Tokens == nil || *got.Tokens != 10 || got.Error != "" {
		t.Fatalf("%+v", got)
	}
}

func TestClaudeFailure(t *testing.T) {
	bin := fixture(t, `cat <<'JSON'
{"type":"system","subtype":"init","session_id":"abc"}
{"type":"result","is_error":true,"result":"sign in"}
JSON
`)
	got := Run(t.Context(), Request{Profile: Profile{Kind: "claude", Executable: bin}, Dir: t.TempDir()}, func(Event) {})
	if got.Error != "sign in" || got.Session != "abc" {
		t.Fatalf("%+v", got)
	}
}

func TestACP(t *testing.T) {
	bin := fixture(t, `read -r initialize
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}'
read -r load
case "$load" in *session/load*abc*) ;; *) exit 9;; esac
echo '{"jsonrpc":"2.0","id":2,"result":{}}'
read -r prompt
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"abc","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}}}}'
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
`)
	got := Run(t.Context(), Request{Profile: Profile{Kind: "hermes", Executable: bin}, Dir: t.TempDir(), Session: "abc"}, func(Event) {})
	if got.Error != "" || got.Text != "done" || got.Session != "abc" {
		t.Fatalf("%+v", got)
	}
}

func TestOutcomeRejectsUnknownAndTrailingData(t *testing.T) {
	var value struct {
		Known string `json:"known"`
	}
	for _, input := range []string{`{"typo":"a"}`, `{"known":"a"} {"known":"b"}`} {
		if DecodeOutcome(input, &value) == nil {
			t.Fatalf("accepted %s", input)
		}
	}
	if err := DecodeOutcome("```json\n{\"known\":\"a\"}\n```", &value); err != nil || value.Known != "a" {
		t.Fatalf("%+v %v", value, err)
	}
}
