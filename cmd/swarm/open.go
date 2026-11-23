package main

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"runtime"
	"time"

	"github.com/mrdavey/pantheon-local/internal/service"
	"github.com/mrdavey/pantheon-local/internal/ui"
)

func openWorkbench(ctx context.Context, dir string) error {
	if _, err := fs.Stat(ui.Files(), "index.html"); err != nil {
		return errors.New("web interface missing: run npm --prefix web run build, then rebuild swarm")
	}
	if info, err := service.Running(ctx, dir); err == nil {
		fmt.Println(info.Link())
		openBrowser(ctx, info)
		return nil
	}
	fmt.Println("Keep this Terminal window open while you use Pantheon Local.")
	err := service.Serve(ctx, dir, "127.0.0.1:0", os.Stdout, func(info service.Info) {
		openBrowser(ctx, info)
	})
	if !errors.Is(err, service.ErrAlreadyRunning) {
		return err
	}
	// The other launch may hold the lock before publishing its session.
	wait, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for wait.Err() == nil {
		if info, err := service.Running(wait, dir); err == nil {
			fmt.Println(info.Link())
			openBrowser(ctx, info)
			return nil
		}
		select {
		case <-wait.Done():
		case <-ticker.C:
		}
	}
	return errors.New("Pantheon Local is running. Use the link in its Terminal window or reopen after it finishes starting.")
}

func openBrowser(ctx context.Context, info service.Info) {
	program := "xdg-open"
	if runtime.GOOS == "darwin" {
		program = "open"
	}
	start, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	if err := exec.CommandContext(start, program, info.Link()).Run(); err != nil {
		fmt.Fprintln(os.Stderr, "Open the Pantheon Local link above in your browser.")
	}
}
