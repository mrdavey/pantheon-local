package run

import (
	"path/filepath"

	"github.com/mrdavey/pantheon-local/internal/routing"
)

func (e *Engine) assign(r *Run, tasks []Task) {
	var catalog routing.Catalog
	if r.Input.Routing != "" && r.Input.Routing != "manual" {
		if err := readJSON(filepath.Join(e.root(*r), "benchmarks.json"), &catalog); err != nil {
			catalog.Note = "The run's benchmark snapshot is unavailable."
		}
	}
	for i := range tasks {
		t := &tasks[i]
		t.Worker, t.Route = routing.Choose(r.Input.Routing, t.Class, t.Worker, r.Input.Workers, r.Profiles, catalog)
		t.State = "pending"
	}
}
