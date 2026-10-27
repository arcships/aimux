// Command ffidownload downloads the matching native archive and generated
// header from one GitHub Release into the paths expected by cgo.
//
// Invoked automatically by `go generate ./...` — pure standard library so it
// works on Linux / macOS / Windows without bash or curl.
//
// The version defaults to "latest"; pin a specific release with:
//
//	AIMUX_FFI_VERSION=v0.1.0 go generate ./...
package main

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
)

// targetTriple maps GOOS/GOARCH to the Rust target used in the GitHub
// Release asset names. Must stay in sync with the go-build matrix in
// .github/workflows/release.yml.
var targetTriple = map[string]string{
	"linux/amd64":   "x86_64-unknown-linux-gnu",
	"darwin/amd64":  "x86_64-apple-darwin",
	"darwin/arm64":  "aarch64-apple-darwin",
	"windows/amd64": "x86_64-pc-windows-gnu",
}

func main() {
	key := runtime.GOOS + "/" + runtime.GOARCH
	triple, ok := targetTriple[key]
	if !ok {
		fmt.Fprintf(os.Stderr, "ffidownload: no prebuilt libaimux_ffi.a for %s\n", key)
		fmt.Fprintf(os.Stderr, "  supported: linux/amd64, darwin/amd64, darwin/arm64, windows/amd64\n")
		fmt.Fprintf(os.Stderr, "  for other platforms, clone the aimux repo and run `cargo build -p aimux-ffi --profile staticlib-release`.\n")
		os.Exit(1)
	}

	version := os.Getenv("AIMUX_FFI_VERSION")
	if version == "" {
		version = "latest"
	}
	asset := "libaimux_ffi-" + triple + ".a"
	releaseURL := "https://github.com/arcships/aimux/releases/latest/download/"
	if version != "latest" {
		releaseURL = "https://github.com/arcships/aimux/releases/download/" + version + "/"
	}

	// go generate runs with cwd = the package directory; the cgo LDFLAGS
	// search ${SRCDIR}/../../target/staticlib-release first (the profile the
	// release archives are built with) and target/release second, so the
	// download goes to the first one and wins over a local cargo build in a
	// source checkout (works in the Go module cache too).
	cwd, err := os.Getwd()
	if err != nil {
		fatal(err)
	}
	destDir := filepath.Join(cwd, "..", "..", "target", "staticlib-release")
	dest := filepath.Join(destDir, "libaimux_ffi.a")

	fmt.Printf("ffidownload: downloading %s (%s)\n", asset, version)
	if err := download(releaseURL+"aimux.h", filepath.Join(cwd, "..", "..", "aimux-ffi", "include", "aimux.h")); err != nil {
		fatal(err)
	}
	if err := download(releaseURL+asset, dest); err != nil {
		fatal(err)
	}
}

func download(url, dest string) error {
	resp, err := http.Get(url)
	if err != nil {
		return fmt.Errorf("download failed: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download failed: HTTP %d for %s", resp.StatusCode, url)
	}

	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(dest), ".aimux-download-*")
	if err != nil {
		return err
	}
	defer f.Close()
	defer os.Remove(f.Name())

	if _, err := io.Copy(f, resp.Body); err != nil {
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	if err := os.Chmod(f.Name(), 0o644); err != nil {
		return err
	}
	if err := os.Rename(f.Name(), dest); err != nil {
		return err
	}
	fmt.Printf("ffidownload: wrote %s\n", dest)
	return nil
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "ffidownload:", err)
	os.Exit(1)
}
