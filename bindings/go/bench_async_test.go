package aimux

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"runtime"
	"runtime/pprof"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/arcships/aimux/bindings/go/generated"
)

const (
	fanOut       = 64
	burstDeltas  = 5000
	callDelay    = 25 * time.Millisecond
	benchRequest = `{"prompt":"hello","options":{"maxRetries":0}}`
	sseDelta     = "data: {\"id\":\"local\",\"model\":\"test-model\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n"
	sseFinish    = "data: {\"id\":\"local\",\"model\":\"test-model\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
)

func loopbackModel(t testing.TB, server http.HandlerFunc) *Handle {
	t.Helper()
	if os.Getenv("AIMUX_TEST_ABI") == "1" {
		t.Skip("requires the Rust library")
	}
	endpoint := httptest.NewServer(server)
	t.Cleanup(func() {
		endpoint.CloseClientConnections()
		endpoint.Close()
	})
	ctx := context.Background()
	provider, err := Create(ctx, nil, generated.OP_PROVIDER_CREATE, map[string]any{
		"package": "deepseek", "settings": map[string]any{"apiKey": "loopback", "baseUrl": endpoint.URL},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { provider.Close() })
	model, err := Create(ctx, provider, generated.OP_PROVIDER_MODEL, map[string]string{
		"method": "languageModel", "modelId": "test-model",
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { model.Close() })
	return model
}

// Sample process threads while all calls are parked at the loopback server.
// The Go threadcreate profile and goroutine count are reported on all hosts.
func processThreads() int {
	pid := strconv.Itoa(os.Getpid())
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("ps", "-M", "-p", pid)
	case "linux":
		cmd = exec.Command("ps", "-L", "-p", pid)
	default:
		return 0
	}
	out, err := cmd.Output()
	if err != nil {
		return 0
	}
	return len(strings.Split(strings.TrimSpace(string(out)), "\n")) - 1
}

func BenchmarkCallFanOut(b *testing.B) {
	for _, mode := range []struct {
		name string
		call func(uint64, string, []byte, uint64) (json.RawMessage, error)
	}{{"sync", nativeCallSync}, {"async", nativeCall}} {
		b.Run(mode.name, func(b *testing.B) {
			const response = `{"id":"local","model":"test-model","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`
			type gate struct {
				arrived chan struct{}
				release chan struct{}
			}
			var warmup atomic.Pointer[gate]
			var requests atomic.Int64
			model := loopbackModel(b, func(w http.ResponseWriter, r *http.Request) {
				if r.Method != http.MethodPost || r.URL.Path != "/chat/completions" {
					http.Error(w, "unexpected request", http.StatusBadRequest)
					return
				}
				requests.Add(1)
				if g := warmup.Load(); g != nil {
					g.arrived <- struct{}{}
					<-g.release
				}
				time.Sleep(callDelay)
				w.Header().Set("Content-Type", "application/json")
				fmt.Fprint(w, response)
			})
			id := model.id.Load()
			run := func() (wait func() time.Duration) {
				var wg sync.WaitGroup
				var elapsed atomic.Int64
				start := make(chan struct{})
				wg.Add(fanOut)
				for range fanOut {
					go func() {
						defer wg.Done()
						<-start
						started := time.Now()
						raw, err := mode.call(id, generated.OP_TEXT_GENERATE, []byte(benchRequest), 0)
						elapsed.Add(int64(time.Since(started)))
						var result struct {
							Steps []struct{ Content []struct{ Text string } }
						}
						if err != nil || json.Unmarshal(raw, &result) != nil ||
							len(result.Steps) != 1 || len(result.Steps[0].Content) != 1 || result.Steps[0].Content[0].Text != "ok" {
							b.Errorf("unexpected generation result: %s, %v", raw, err)
						}
					}()
				}
				close(start)
				return func() time.Duration { wg.Wait(); return time.Duration(elapsed.Load()) }
			}
			g := &gate{make(chan struct{}, fanOut), make(chan struct{})}
			warmup.Store(g)
			wait := run()
			deadline := time.NewTimer(5 * time.Second)
			for range fanOut {
				select {
				case <-g.arrived:
				case <-deadline.C:
					close(g.release)
					wait()
					b.Fatal("warmup failed to park all 64 calls")
				}
			}
			deadline.Stop()
			threads, goThreads, goroutines := processThreads(), pprof.Lookup("threadcreate").Count(), runtime.NumGoroutine()
			warmup.Store(nil)
			close(g.release)
			wait()
			startCgo := runtime.NumCgoCall()
			b.ResetTimer()
			var latency time.Duration
			for range b.N {
				latency += run()()
			}
			b.StopTimer()
			calls := b.N * fanOut
			if requests.Load() != int64(calls+fanOut) {
				b.Fatalf("mock server received %d requests, want %d", requests.Load(), calls+fanOut)
			}
			b.ReportMetric(float64(calls)/b.Elapsed().Seconds(), "calls/s")
			b.ReportMetric(float64(latency.Microseconds())/float64(calls), "mean-us/call")
			if threads > 0 {
				b.ReportMetric(float64(threads), "os-threads")
			}
			b.ReportMetric(float64(goThreads), "go-threads")
			b.ReportMetric(float64(goroutines), "goroutines")
			b.ReportMetric(float64(runtime.NumCgoCall()-startCgo)/float64(calls), "cgocalls/call")
		})
	}
}

func BenchmarkStreamBurst(b *testing.B) {
	for _, mode := range []string{"per_frame", "batch"} {
		b.Run(mode, func(b *testing.B) {
			payload := strings.Repeat(sseDelta, burstDeltas) + sseFinish
			model := loopbackModel(b, func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				fmt.Fprint(w, payload)
			})
			id := model.id.Load()
			var totalFrames, totalCallbacks int
			b.ResetTimer()
			for range b.N {
				var events, deltas int
				s := &Stream{cancel: func() {}, emit: func(raw json.RawMessage) bool {
					events++
					if bytes.Contains(raw, []byte(`"type":"text-delta"`)) {
						deltas++
					}
					return true
				}}
				callbacks := &streamCallbacks{stream: s, done: make(chan struct{})}
				var err error
				if mode == "per_frame" {
					err = nativeStreamSync(id, generated.OP_TEXT_STREAM, []byte(benchRequest), 0, callbacks)
				} else {
					err = nativeStream(id, generated.OP_TEXT_STREAM, []byte(benchRequest), 0, callbacks, make(chan error, 1))
				}
				if err != nil || s.failure != nil || s.terminal == nil || s.terminal.Outcome != "success" || deltas != burstDeltas {
					b.Fatalf("burst lost data: frames=%d deltas=%d callbacks=%d terminal=%+v errors=%v, %v", events, deltas, callbacks.calls, s.terminal, err, s.failure)
				}
				totalFrames += events + 1
				totalCallbacks += callbacks.calls
			}
			b.StopTimer()
			b.ReportMetric(float64(totalFrames)/b.Elapsed().Seconds(), "frames/s")
			b.ReportMetric(float64(totalFrames)/float64(b.N), "frames/stream")
			b.ReportMetric(float64(totalCallbacks)/float64(b.N), "callbacks/stream")
		})
	}
}
