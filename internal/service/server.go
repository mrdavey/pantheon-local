package service

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/mrdavey/pantheon-local/internal/adapter"
	"github.com/mrdavey/pantheon-local/internal/eval"
	"github.com/mrdavey/pantheon-local/internal/run"
	"github.com/mrdavey/pantheon-local/internal/ui"
)

type Setup struct {
	Profiles    map[string]adapter.Profile `json:"profiles"`
	Coordinator string                     `json:"coordinator,omitempty"`
	Workers     []string                   `json:"workers,omitempty"`
}

var ErrAlreadyRunning = errors.New("Pantheon Local is already running for this state directory")

type Info struct {
	URL   string `json:"url"`
	Token string `json:"token"`
}

func StateDir() (string, error) {
	if path := os.Getenv("SWARM_STATE_DIR"); path != "" {
		return filepath.Abs(path)
	}
	path, err := os.UserConfigDir()
	return filepath.Join(path, "local-swarm"), err
}

func ReadSetup(dir string) (Setup, error) {
	p := Setup{Profiles: map[string]adapter.Profile{"codex": {Kind: "codex"}, "claude": {Kind: "claude"}, "hermes": {Kind: "hermes"}, "antigravity": {Kind: "antigravity"}}}
	raw, err := os.ReadFile(filepath.Join(dir, "profiles.json"))
	if err == nil {
		p = Setup{}
		if err = json.Unmarshal(raw, &p); err != nil {
			return p, err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return p, err
	}
	if prefs, err := os.ReadFile(filepath.Join(dir, "roles.json")); err == nil {
		var choice Setup
		if json.Unmarshal(prefs, &choice) == nil {
			if _, ok := p.Profiles[choice.Coordinator]; ok {
				p.Coordinator = choice.Coordinator
			}
			p.Workers = nil
			for _, name := range choice.Workers {
				if _, ok := p.Profiles[name]; ok {
					p.Workers = append(p.Workers, name)
				}
			}
		}
	}
	return p, nil
}

type Server struct {
	engine    *run.Engine
	dir       string
	token     string
	baseURL   string
	ctx       context.Context
	mu        sync.Mutex
	profileMu sync.Mutex
}

func Serve(ctx context.Context, dir, listen string, announce io.Writer, ready func(Info)) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	lock, err := os.OpenFile(filepath.Join(dir, "daemon.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		if errors.Is(err, syscall.EWOULDBLOCK) {
			return ErrAlreadyRunning
		}
		return err
	}
	listener, err := net.Listen("tcp4", listen)
	if err != nil {
		return err
	}
	defer listener.Close()
	if !listener.Addr().(*net.TCPAddr).IP.IsLoopback() {
		return errors.New("use a loopback listen address")
	}
	db, err := run.Open(dir)
	if err != nil {
		return err
	}
	defer db.Close()
	raw := make([]byte, 32)
	if _, err = rand.Read(raw); err != nil {
		return err
	}
	token := hex.EncodeToString(raw)
	info := Info{URL: "http://" + listener.Addr().String(), Token: token}
	meta := filepath.Join(dir, "service.json")
	if err = writeLocal(meta, info); err != nil {
		return err
	}
	defer os.Remove(meta)
	server := &Server{engine: db, dir: dir, token: token, baseURL: info.URL, ctx: ctx}
	httpServer := &http.Server{Handler: server.routes(), ReadHeaderTimeout: 5 * time.Second, ReadTimeout: 25 * time.Second}
	loopContext, stopLoop := context.WithCancel(ctx)
	defer stopLoop()
	done := make(chan struct{})
	go func() { defer close(done); db.Loop(loopContext, os.Stderr) }()
	go func() {
		<-ctx.Done()
		_ = httpServer.Close()
	}()
	fmt.Fprintln(announce, info.Link())
	if ready != nil {
		go ready(info)
	}
	err = httpServer.Serve(listener)
	stopLoop()
	<-done
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

func (s *Server) routes() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/setup", s.setup)
	mux.HandleFunc("GET /api/destinations", func(w http.ResponseWriter, r *http.Request) {
		names, err := s.engine.Destinations()
		result(w, names, err)
	})
	mux.HandleFunc("GET /api/runs/{id}/history", func(w http.ResponseWriter, r *http.Request) {
		before, _ := strconv.ParseInt(r.URL.Query().Get("before"), 10, 64)
		history, err := s.engine.History(r.PathValue("id"), before)
		result(w, history, err)
	})
	mux.HandleFunc("GET /api/runs/{id}/cases", func(w http.ResponseWriter, r *http.Request) {
		before, _ := strconv.ParseInt(r.URL.Query().Get("before"), 10, 64)
		cases, err := s.engine.Cases(r.PathValue("id"), before)
		result(w, cases, err)
	})
	mux.HandleFunc("GET /api/runs/{id}/evidence/{candidate}", func(w http.ResponseWriter, r *http.Request) {
		report, err := s.engine.Evidence(r.PathValue("id"), r.PathValue("candidate"))
		result(w, report, err)
	})
	mux.HandleFunc("POST /api/runs/{id}/blind/{pair}/{side}", func(w http.ResponseWriter, r *http.Request) {
		candidate, err := s.engine.PreferenceCandidate(r.PathValue("id"), r.PathValue("pair"), r.PathValue("side"))
		if err != nil {
			result(w, nil, err)
			return
		}
		r.SetPathValue("candidate", candidate)
		s.preview(w, r)
	})
	mux.HandleFunc("GET /api/setup/status", s.setupStatus)
	mux.HandleFunc("POST /api/setup/model", s.setupModel)
	mux.HandleFunc("GET /api/session", func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusNoContent) })
	mux.HandleFunc("GET /api/snapshot", func(w http.ResponseWriter, _ *http.Request) {
		view, err := s.engine.Snapshot()
		for i := range view.Runs {
			view.Runs[i] = view.Runs[i].Public()
		}
		result(w, view, err)
	})
	mux.HandleFunc("GET /api/runs", s.list)
	mux.HandleFunc("POST /api/runs", s.start)
	mux.HandleFunc("GET /api/runs/{id}", s.get)
	mux.HandleFunc("POST /api/runs/{id}", s.command)
	mux.HandleFunc("GET /api/runs/{id}/turns/{turn}", s.log)
	mux.HandleFunc("POST /api/runs/{id}/preview/{candidate}", s.preview)
	mux.HandleFunc("GET /api/runs/{id}/diff/{candidate}", func(w http.ResponseWriter, r *http.Request) {
		value, err := s.engine.Diff(r.Context(), r.PathValue("id"), r.PathValue("candidate"))
		result(w, value, err)
	})
	mux.HandleFunc("GET /api/events", s.events)
	mux.HandleFunc("POST /api/advice", s.advice)
	mux.HandleFunc("GET /", func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/" {
			if _, err := fs.Stat(ui.Files(), "index.html"); err != nil {
				http.Error(w, "Build the web interface before launching the workbench.", http.StatusServiceUnavailable)
				return
			}
		}
		http.FileServer(http.FS(ui.Files())).ServeHTTP(w, r)
	})
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != strings.TrimPrefix(s.baseURL, "http://") {
			http.Error(w, "unexpected local host", http.StatusBadRequest)
			return
		}
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Content-Security-Policy", "default-src 'self'; style-src 'self'; object-src 'none'; frame-ancestors 'none'")
		if strings.HasPrefix(r.URL.Path, "/api/") {
			if origin := r.Header.Get("Origin"); origin != "" && origin != s.baseURL {
				http.Error(w, "unexpected origin", http.StatusForbidden)
				return
			}
			if subtle.ConstantTimeCompare([]byte(r.Header.Get("Authorization")), []byte("Bearer "+s.token)) != 1 {
				http.Error(w, "authentication required", http.StatusUnauthorized)
				return
			}
			w.Header().Set("Cache-Control", "no-store")
		}
		mux.ServeHTTP(w, r)
	})
}

func result(w http.ResponseWriter, value any, err error) {
	w.Header().Set("Content-Type", "application/json")
	if err != nil {
		status := http.StatusBadRequest
		if errors.Is(err, run.ErrStaleRevision) {
			status = http.StatusConflict
		}
		w.WriteHeader(status)
		value = map[string]string{"error": err.Error()}
	}
	_ = json.NewEncoder(w).Encode(value)
}
func decode(w http.ResponseWriter, r *http.Request, value any) error {
	d := json.NewDecoder(http.MaxBytesReader(w, r.Body, 1024*1024))
	d.DisallowUnknownFields()
	return d.Decode(value)
}

func (s *Server) setup(w http.ResponseWriter, _ *http.Request) {
	p, err := ReadSetup(s.dir)
	result(w, p, err)
}
func (s *Server) list(w http.ResponseWriter, _ *http.Request) {
	r, err := s.engine.List()
	for i := range r {
		r[i] = r[i].Public()
	}
	result(w, r, err)
}
func (s *Server) get(w http.ResponseWriter, r *http.Request) {
	v, err := s.engine.Get(r.PathValue("id"))
	result(w, v.Public(), err)
}
func (s *Server) start(w http.ResponseWriter, r *http.Request) {
	var input run.Start
	if err := decode(w, r, &input); err != nil {
		result(w, nil, err)
		return
	}
	cfg, err := ReadSetup(s.dir)
	if err != nil {
		result(w, nil, err)
		return
	}
	v, err := s.engine.Start(r.Context(), input, cfg.Profiles)
	if err == nil {
		if saveErr := writeLocal(filepath.Join(s.dir, "roles.json"), Setup{Coordinator: input.Coordinator, Workers: input.Workers}); saveErr != nil {
			fmt.Fprintln(os.Stderr, saveErr)
		}
	}
	result(w, v.Public(), err)
}
func (s *Server) command(w http.ResponseWriter, r *http.Request) {
	var input run.Command
	if err := decode(w, r, &input); err != nil {
		result(w, nil, err)
		return
	}
	v, err := s.engine.Command(r.Context(), r.PathValue("id"), input)
	result(w, v.Public(), err)
}
func (s *Server) log(w http.ResponseWriter, r *http.Request) {
	b, err := s.engine.Log(r.PathValue("id"), r.PathValue("turn"))
	result(w, string(b), err)
}
func (s *Server) events(w http.ResponseWriter, r *http.Request) {
	after, _ := strconv.ParseInt(r.URL.Query().Get("after"), 10, 64)
	v, err := s.engine.Events(after)
	for i := range v {
		v[i].Run = v[i].Run.Public()
	}
	result(w, v, err)
}
func (s *Server) advice(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Profile string `json:"profile"`
		Prompt  string `json:"prompt"`
	}
	if err := decode(w, r, &input); err != nil {
		result(w, nil, err)
		return
	}
	cfg, err := ReadSetup(s.dir)
	if err != nil {
		result(w, nil, err)
		return
	}
	p, ok := cfg.Profiles[input.Profile]
	if !ok {
		result(w, nil, errors.New("select an advisor profile"))
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Minute)
	defer cancel()
	v, err := s.engine.Advice(ctx, p, input.Prompt)
	result(w, v, err)
}

func (s *Server) preview(w http.ResponseWriter, r *http.Request) {
	path, args, err := s.engine.Preview(r.Context(), r.PathValue("id"), r.PathValue("candidate"))
	if err != nil {
		result(w, nil, err)
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	port, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		result(w, nil, err)
		return
	}
	addr := port.Addr().(*net.TCPAddr)
	_ = port.Close()
	data, err := os.MkdirTemp(s.dir, "preview-")
	if err != nil {
		result(w, nil, err)
		return
	}
	current, err := s.engine.Get(r.PathValue("id"))
	if err != nil {
		result(w, nil, err)
		return
	}
	cmd := eval.Command(s.ctx, path, data, args, eval.Scope{Repository: current.Repo.Root, State: s.dir})
	cmd.Env = append(cmd.Env, "SWARM_PORT="+strconv.Itoa(addr.Port), "PORT="+strconv.Itoa(addr.Port), "HOST=127.0.0.1")
	log, err := os.Create(filepath.Join(data, "preview.log"))
	if err != nil {
		result(w, nil, err)
		return
	}
	cmd.Stdout, cmd.Stderr = log, log
	if err = cmd.Start(); err != nil {
		_ = log.Close()
		result(w, nil, err)
		return
	}
	closed := make(chan error, 1)
	go func() { closed <- cmd.Wait(); _ = log.Close() }()
	ticker := time.NewTicker(200 * time.Millisecond)
	defer ticker.Stop()
	ctx, cancel := context.WithTimeout(r.Context(), 12*time.Second)
	defer cancel()
	url := "http://" + addr.String()
	for {
		select {
		case err := <-closed:
			result(w, nil, fmt.Errorf("preview exited: %v; see %s", err, log.Name()))
			return
		case <-ctx.Done():
			_ = cmd.Process.Kill()
			result(w, nil, fmt.Errorf("preview did not listen on SWARM_PORT; see %s", log.Name()))
			return
		case <-ticker.C:
			conn, err := net.DialTimeout("tcp4", addr.String(), 100*time.Millisecond)
			if err == nil {
				_ = conn.Close()
				result(w, map[string]string{"url": url}, nil)
				return
			}
		}
	}
}

func writeLocal(path string, value any) error {
	file, err := os.CreateTemp(filepath.Dir(path), ".swarm-local-")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	err = json.NewEncoder(file).Encode(value)
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(file.Name(), path)
}
