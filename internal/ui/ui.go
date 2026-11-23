package ui

import (
	"embed"
	"io/fs"
)

//go:embed all:dist
var files embed.FS

func Files() fs.FS {
	root, _ := fs.Sub(files, "dist")
	return root
}
