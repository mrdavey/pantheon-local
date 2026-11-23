package adapter

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"strings"
	"sync"
)

type Profile struct {
	BenchmarkModel  string `json:"benchmark_model,omitempty"`
	BenchmarkAgent  string `json:"benchmark_agent,omitempty"`
	BenchmarkEffort string `json:"benchmark_effort,omitempty"`
	Kind            string `json:"kind"`
	Executable      string `json:"executable,omitempty"`
	Model           string `json:"model,omitempty"`
	HermesProfile   string `json:"hermes_profile,omitempty"`
	Billing         string `json:"billing,omitempty"`
}

func (p Profile) Binary() string {
	if p.Executable != "" {
		return p.Executable
	}
	if p.Kind == "antigravity" {
		return "agy"
	}
	if p.Kind == "claude" {
		return "claude"
	}
	return p.Kind
}

func (p Profile) Validate() error {
	switch p.Kind {
	case "codex", "claude", "hermes", "antigravity", "custom":
	default:
		return fmt.Errorf("unknown adapter %q", p.Kind)
	}
	if p.Kind == "custom" && p.Executable == "" {
		return errors.New("custom adapter needs an executable")
	}
	return nil
}

type Request struct {
	Environment []string `json:"-"`
	Profile     Profile  `json:"profile"`
	Role        string   `json:"role"`
	Dir         string   `json:"dir"`
	Prompt      string   `json:"prompt"`
	Session     string   `json:"session,omitempty"`
}

type Event struct {
	Type    string          `json:"type"`
	Session string          `json:"session,omitempty"`
	Text    string          `json:"text,omitempty"`
	Raw     json.RawMessage `json:"raw,omitempty"`
}

type Result struct {
	Session string `json:"session,omitempty"`
	Text    string `json:"text,omitempty"`
	Tokens  *int64 `json:"tokens,omitempty"`
	Error   string `json:"error,omitempty"`
}

func Probe(ctx context.Context, p Profile) (string, error) {
	if err := p.Validate(); err != nil {
		return "", err
	}
	out, err := exec.CommandContext(ctx, p.Binary(), "--version").CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("%s: %w", p.Binary(), err)
	}
	return strings.TrimSpace(string(out)), nil
}

type usage struct {
	Input    int64 `json:"input_tokens"`
	Output   int64 `json:"output_tokens"`
	Creation int64 `json:"cache_creation_input_tokens"`
	Read     int64 `json:"cache_read_input_tokens"`
}

type nativeEvent struct {
	Type    string `json:"type"`
	Thread  string `json:"thread_id"`
	Session string `json:"session_id"`
	Error   bool   `json:"is_error"`
	Result  string `json:"result"`
	Usage   *usage `json:"usage"`
	Item    struct {
		Type string `json:"type"`
		Text string `json:"text"`
	} `json:"item"`
}

func nativeArgs(r Request) []string {
	if r.Profile.Kind == "codex" {
		args := []string{"exec"}
		if r.Session != "" {
			args = append(args, "resume")
		}
		args = append(args, "--json")
		if r.Role == "advisor" || r.Role == "chat" || r.Role == "observer" {
			args = append(args, "--skip-git-repo-check")
		}
		if r.Session == "" {
			mode := "read-only"
			if r.Role == "worker" {
				mode = "workspace-write"
			}
			args = append(args, "--sandbox", mode)
		}
		if r.Profile.Model != "" {
			args = append(args, "--model", r.Profile.Model)
		}
		if r.Session != "" {
			args = append(args, r.Session)
		}
		return append(args, "-")
	}
	args := []string{"-p", "--verbose", "--output-format", "stream-json"}
	if r.Role != "worker" {
		args = append(args, "--tools", "")
	}
	if r.Profile.Model != "" {
		args = append(args, "--model", r.Profile.Model)
	}
	if r.Session != "" {
		args = append(args, "--resume", r.Session)
	}
	return args
}

func Run(ctx context.Context, r Request, emit func(Event)) Result {
	if err := r.Profile.Validate(); err != nil {
		return Result{Error: err.Error()}
	}
	switch r.Profile.Kind {
	case "hermes", "antigravity":
		return runACP(ctx, r, emit)
	case "custom":
		return runCustom(ctx, r, emit)
	}
	cmd := exec.CommandContext(ctx, r.Profile.Binary(), nativeArgs(r)...)
	cmd.Env = r.Environment
	cmd.Dir, cmd.Stdin = r.Dir, strings.NewReader(r.Prompt)
	return readProcess(cmd, r.Session, emit, func(line []byte, result *Result) bool {
		var event nativeEvent
		if json.Unmarshal(line, &event) != nil || event.Type == "" {
			return false
		}
		if session := first(event.Thread, event.Session); session != "" {
			result.Session = session
		}
		if event.Item.Type == "agent_message" && event.Item.Text != "" {
			result.Text = event.Item.Text
		}
		if event.Type == "result" && event.Result != "" {
			result.Text = event.Result
		}
		if event.Error || event.Type == "turn.failed" {
			result.Error = first(event.Result, "native turn failed")
		}
		if event.Usage != nil && (event.Type == "result" || event.Type == "turn.completed") {
			u := event.Usage
			n := u.Input + u.Output
			if r.Profile.Kind == "claude" {
				n += u.Creation + u.Read
			}
			result.Tokens = &n
		}
		message := event.Item.Text
		if event.Type == "result" {
			message = event.Result
		}
		emit(Event{Type: event.Type, Session: result.Session, Text: message, Raw: bytes.Clone(line)})
		return true
	})
}

func readProcess(cmd *exec.Cmd, session string, emit func(Event), parse func([]byte, *Result) bool) Result {
	result := Result{Session: session}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return Result{Error: err.Error()}
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return Result{Error: err.Error()}
	}
	if err := cmd.Start(); err != nil {
		return Result{Error: err.Error()}
	}
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		reader := bufio.NewReader(stderr)
		for {
			line, err := reader.ReadSlice('\n')
			if len(line) > 0 {
				emit(Event{Type: "stderr", Text: strings.ToValidUTF8(string(line), "")})
			}
			if err != nil && !errors.Is(err, bufio.ErrBufferFull) {
				return
			}
		}
	}()
	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 64*1024), 8*1024*1024)
	for scanner.Scan() {
		if !parse(scanner.Bytes(), &result) {
			emit(Event{Type: "stdout", Text: scanner.Text()})
		}
	}
	if err := scanner.Err(); err != nil {
		result.Error = err.Error()
		_ = cmd.Process.Kill()
	}
	wg.Wait()
	if err := cmd.Wait(); err != nil && result.Error == "" {
		result.Error = err.Error()
	}
	if result.Text == "" && result.Error == "" {
		result.Error = "adapter returned no final response"
	}
	return result
}

func first(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func runCustom(ctx context.Context, r Request, emit func(Event)) Result {
	payload, err := json.Marshal(r)
	if err != nil {
		return Result{Error: err.Error()}
	}
	cmd := exec.CommandContext(ctx, r.Profile.Binary(), "turn-v1")
	cmd.Env = r.Environment
	cmd.Dir, cmd.Stdin = r.Dir, bytes.NewReader(append(payload, '\n'))
	return readProcess(cmd, r.Session, emit, func(line []byte, result *Result) bool {
		var message struct {
			Event  *Event  `json:"event"`
			Result *Result `json:"result"`
		}
		if json.Unmarshal(line, &message) != nil {
			return false
		}
		if message.Event != nil {
			if message.Event.Session != "" {
				result.Session = message.Event.Session
			}
			emit(*message.Event)
			return true
		}
		if message.Result != nil {
			*result = *message.Result
			return true
		}
		return false
	})
}

// DecodeOutcome accepts a JSON document or a single fenced JSON document.
func DecodeOutcome(text string, value any) error {
	text = strings.TrimSpace(text)
	if strings.HasPrefix(text, "```json\n") && strings.HasSuffix(text, "\n```") {
		text = strings.TrimSuffix(strings.TrimPrefix(text, "```json\n"), "\n```")
	}
	decoder := json.NewDecoder(strings.NewReader(text))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(value); err != nil {
		return err
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return errors.New("expected one JSON document")
	}
	return nil
}
