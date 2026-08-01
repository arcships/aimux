// Command ffidownload downloads the prebuilt libaimux_ffi.a for the current
// platform from the GitHub Release assets and places it where the cgo
// LDFLAGS expect it (<module>/target/release/libaimux_ffi.a).
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
		fmt.Fprintf(os.Stderr, "  for other platforms, clone the aimux repo and run `cargo build -p aimux-ffi --release`.\n")
		os.Exit(1)
	}

	version := os.Getenv("AIMUX_FFI_VERSION")
	if version == "" {
		version = "latest"
	}
	asset := "libaimux_ffi-" + triple + ".a"
	url := fmt.Sprintf("https://github.com/arcships/aimux/releases/%s/download/%s", version, asset)

	// go generate runs with cwd = the package directory; the cgo LDFLAGS use
	// ${SRCDIR}/../../target/release, so write exactly there (works both in
	// the Go module cache and in a source checkout).
	cwd, err := os.Getwd()
	if err != nil {
		fatal(err)
	}
	destDir := filepath.Join(cwd, "..", "..", "target", "release")
	dest := filepath.Join(destDir, "libaimux_ffi.a")

	fmt.Printf("ffidownload: downloading %s (%s)\n", asset, version)
	resp, err := http.Get(url)
	if err != nil {
		fatal(fmt.Errorf("download failed: %w", err))
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		fatal(fmt.Errorf("download failed: HTTP %d for %s", resp.StatusCode, url))
	}

	if err := os.MkdirAll(destDir, 0o755); err != nil {
		fatal(err)
	}
	f, err := os.Create(dest)
	if err != nil {
		fatal(err)
	}
	defer f.Close()

	if _, err := io.Copy(f, resp.Body); err != nil {
		fatal(err)
	}
	fmt.Printf("ffidownload: wrote %s\n", dest)
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "ffidownload:", err)
	os.Exit(1)
}
