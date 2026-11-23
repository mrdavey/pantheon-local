package adapter

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"strings"
	"sync"
)

type acpMessage struct {
	ID     json.RawMessage `json:"id,omitempty"`
	Method string          `json:"method,omitempty"`
	Params json.RawMessage `json:"params,omitempty"`
	Result json.RawMessage `json:"result,omitempty"`
	Error  *struct {
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

type acpClient struct {
	lines   *bufio.Scanner
	out     *json.Encoder
	emit    func(Event)
	text    strings.Builder
	session string
	id      int
}

func (c *acpClient) call(method string, params any, result any) error {
	c.id++
	id := fmt.Sprint(c.id)
	if err := c.out.Encode(map[string]any{"jsonrpc": "2.0", "id": c.id, "method": method, "params": params}); err != nil {
		return err
	}
	for c.lines.Scan() {
		var message acpMessage
		if err := json.Unmarshal(c.lines.Bytes(), &message); err != nil {
			continue
		}
		if message.Method != "" {
			if err := c.handle(message); err != nil {
				return err
			}
			continue
		}
		if string(message.ID) != id {
			continue
		}
		if message.Error != nil {
			return errors.New(message.Error.Message)
		}
		if result == nil {
			return nil
		}
		return json.Unmarshal(message.Result, result)
	}
	return firstError(c.lines.Err(), io.ErrUnexpectedEOF)
}

func firstError(err, fallback error) error {
	if err != nil {
		return err
	}
	return fallback
}

func (c *acpClient) handle(m acpMessage) error {
	if len(m.ID) > 0 && string(m.ID) != "null" {
		response := map[string]any{"jsonrpc": "2.0", "id": m.ID}
		if m.Method == "session/request_permission" {
			response["result"] = map[string]any{"outcome": map[string]string{"outcome": "cancelled"}}
		} else {
			response["error"] = map[string]any{"code": -32601, "message": "Host capability unavailable"}
		}
		c.emit(Event{Type: m.Method, Session: c.session, Raw: m.Params})
		return c.out.Encode(response)
	}
	var data struct {
		Update struct {
			Kind    string `json:"sessionUpdate"`
			Content struct {
				Text string `json:"text"`
			} `json:"content"`
		} `json:"update"`
	}
	_ = json.Unmarshal(m.Params, &data)
	if data.Update.Kind == "agent_message_chunk" {
		c.text.WriteString(data.Update.Content.Text)
	}
	c.emit(Event{Type: first(data.Update.Kind, m.Method), Session: c.session, Text: data.Update.Content.Text, Raw: m.Params})
	return nil
}

func runACP(ctx context.Context, r Request, emit func(Event)) (result Result) {
	var args []string
	if r.Profile.Kind == "hermes" && r.Profile.HermesProfile != "" {
		args = append(args, "--profile", r.Profile.HermesProfile)
	}
	args = append(args, "acp")
	cmd := exec.CommandContext(ctx, r.Profile.Binary(), args...)
	cmd.Env = r.Environment
	cmd.Dir = r.Dir
	in, err := cmd.StdinPipe()
	if err != nil {
		return Result{Error: err.Error()}
	}
	out, err := cmd.StdoutPipe()
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
		scan := bufio.NewScanner(stderr)
		for scan.Scan() {
			emit(Event{Type: "stderr", Text: scan.Text()})
		}
		_, _ = io.Copy(io.Discard, stderr)
	}()
	defer func() { _ = in.Close(); _ = cmd.Process.Kill(); wg.Wait(); _ = cmd.Wait() }()
	scan := bufio.NewScanner(out)
	scan.Buffer(make([]byte, 64*1024), 8*1024*1024)
	client := acpClient{lines: scan, out: json.NewEncoder(in), emit: emit}
	fail := func(err error) Result { return Result{Session: client.session, Error: err.Error()} }
	var init struct {
		Capabilities struct {
			Load bool `json:"loadSession"`
		} `json:"agentCapabilities"`
	}
	if err := client.call("initialize", map[string]any{"protocolVersion": 1, "clientCapabilities": map[string]any{}, "clientInfo": map[string]string{"name": "local-swarm", "version": "0.1.0"}}, &init); err != nil {
		return fail(err)
	}
	params := map[string]any{"cwd": r.Dir, "mcpServers": []any{}}
	if r.Session == "" {
		var newSession struct {
			Session string `json:"sessionId"`
		}
		if err := client.call("session/new", params, &newSession); err != nil {
			return fail(err)
		}
		client.session = newSession.Session
	} else {
		client.session = r.Session
		params["sessionId"] = r.Session
		method := "session/resume"
		if init.Capabilities.Load {
			method = "session/load"
		}
		if err := client.call(method, params, nil); err != nil {
			return fail(err)
		}
	}
	if client.session == "" {
		return fail(errors.New("ACP returned no session ID"))
	}
	emit(Event{Type: "session", Session: client.session})
	client.text.Reset()
	var prompt struct {
		Stop string `json:"stopReason"`
	}
	if err := client.call("session/prompt", map[string]any{"sessionId": client.session, "prompt": []map[string]string{{"type": "text", "text": r.Prompt}}}, &prompt); err != nil {
		return fail(err)
	}
	text := client.text.String()
	if prompt.Stop != "end_turn" || text == "" {
		return fail(fmt.Errorf("ACP turn stopped: %s", prompt.Stop))
	}
	return Result{Session: client.session, Text: text}
}
