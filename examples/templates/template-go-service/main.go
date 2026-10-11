// Command template-go-service — **a network service on a Rivet, in Go.**
//
// `ADR-036` §6.2. The companion to `examples/templates/template-network-service`, which
// is the same idea in Rust. Read `rivet.go` first: it is the contract, and it is
// the part worth copying.
//
// # What this proves that the Rust template does not
//
//  1. **The contract is language-neutral.** Nothing here links `rivetos-sdk`;
//     it cannot. Everything a Rivet hands an application is read from the same
//     promises, in another language, in one short file.
//  2. **Two descriptors, told apart by name.** This declares a TCP `listen`
//     AND a `provides` socket, so the supervisor hands over two and the program
//     picks by `LISTEN_FDNAMES`. Nothing had exercised that.
//  3. **Go builds for a Rivet with no container and no C toolchain** —
//     `CGO_ENABLED=0` makes a cross-compile two environment variables.
//
// # What it does, which is deliberately dull
//
// It counts ticks, persists the count under `RIVET_APP_DATA` so it survives a
// restart, and reports it two ways: as JSON on the endpoint it *provides* for
// another application, and over HTTP for a person with a browser.
//
// That shape is Aura's — a controller holding state and serving a consumer over
// a unix socket — with none of Aura's content.
package main

import (
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"
	"time"
)

// service is the name in the manifest, the journal, and the supervisor's
// service list. `cargo rivet package` derives it from `[package] name`.
const service = "template-go-service"

// The names the supervisor gives the two descriptors. `tcp-<port>` for a
// `listen`, and the endpoint's own name for a `provides` — see rivet.go.
const (
	httpSocketName = "tcp-8090"
	apiSocketName  = "api"
)

type state struct {
	ticks   atomic.Int64
	started time.Time
	dataDir string
	j       *journal
}

func main() {
	if err := run(); err != nil {
		// A failure before the journal exists still has to be said. Under a
		// supervisor there is no console, so stderr is what gets captured.
		fmt.Fprintf(os.Stderr, "%s: %v\n", service, err)
		os.Exit(1)
	}
}

func run() error {
	j := newJournal(service)

	// ---- what this service was handed ----------------------------------
	dataDir, err := appData()
	if err != nil {
		return err
	}
	if _, err := appDir(); err != nil {
		// Not fatal here — this program ships no assets — but reported,
		// because its absence means the environment is not what it claims.
		j.warn("no application directory was handed over", map[string]any{"why": err.Error()})
	}

	handed, err := listeners()
	if err != nil {
		return err
	}
	if len(handed) == 0 {
		return fmt.Errorf(
			"no listening sockets were handed over. This program declares a `listen` " +
				"and a `provides` and expects a Rivet supervisor to open both " +
				"(LISTEN_FDS was not set, or named another process)")
	}

	httpLn, okHTTP := handed[httpSocketName]
	apiLn, okAPI := handed[apiSocketName]
	if !okHTTP || !okAPI {
		// Named precisely, because "a socket is missing" sends somebody to the
		// wrong file. The manifest and the code have to agree and this is where
		// that disagreement surfaces.
		var got []string
		for name := range handed {
			got = append(got, name)
		}
		return fmt.Errorf("expected descriptors named %q and %q, was handed %v",
			httpSocketName, apiSocketName, got)
	}

	s := &state{started: time.Now(), dataDir: dataDir, j: j}
	s.ticks.Store(readTicks(dataDir, j))

	j.info("starting", map[string]any{
		"data":     dataDir,
		"http":     httpSocketName,
		"provides": apiSocketName,
		"ticks":    s.ticks.Load(),
	})

	// ---- serve ----------------------------------------------------------
	stopping := make(chan os.Signal, 1)
	// SIGTERM is what a supervisor sends; SIGINT is a person at a terminal.
	// SIGHUP is deliberately NOT a stop — it conventionally means reload.
	signal.Notify(stopping, syscall.SIGTERM, syscall.SIGINT)

	go s.tick()
	go s.serveEndpoint(apiLn)

	mux := http.NewServeMux()
	mux.HandleFunc("/", s.index)
	mux.HandleFunc("/status", s.status)
	server := &http.Server{Handler: mux}
	go func() {
		if err := server.Serve(httpLn); err != nil && err != http.ErrServerClosed {
			j.error("the http listener stopped", map[string]any{"why": err.Error()})
		}
	}()

	j.info("listening", nil)

	sig := <-stopping
	j.info("stopping", map[string]any{"signal": sig.String()})
	_ = server.Close()
	_ = apiLn.Close()
	s.writeTicks()
	j.info("stopped", map[string]any{"ticks": s.ticks.Load()})
	return nil
}

// tick counts, and persists every ten so a power cut costs at most ten.
func (s *state) tick() {
	for range time.Tick(time.Second) {
		if s.ticks.Add(1)%10 == 0 {
			s.writeTicks()
		}
	}
}

func (s *state) snapshot() map[string]any {
	return map[string]any{
		"service":  service,
		"uptime_s": int64(time.Since(s.started).Seconds()),
		"ticks":    s.ticks.Load(),
		"message":  "handed two sockets and told where to write",
	}
}

// serveEndpoint answers the socket this application PROVIDES.
//
// One JSON object per connection, then close — the smallest protocol that is
// still a protocol. The consumer is another application the operator granted
// this endpoint to; the supervisor chowns the socket file to that uid and to
// no other (`ADR-015` §5.3, one consumer per endpoint).
func (s *state) serveEndpoint(l net.Listener) {
	for {
		c, err := l.Accept()
		if err != nil {
			return // the listener was closed on the way out
		}
		go func(c net.Conn) {
			defer c.Close()
			_ = c.SetWriteDeadline(time.Now().Add(5 * time.Second))
			_ = json.NewEncoder(c).Encode(s.snapshot())
		}(c)
	}
}

func (s *state) index(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("content-type", "text/plain; charset=utf-8")
	fmt.Fprint(w, strings.Join([]string{
		service + " - a Rivet network service template, in Go",
		"",
		"  GET /status   the same JSON this service offers on its `provides` socket",
		"",
		"It was handed TWO sockets and told where to write.",
		"See docs/ADR-036-The-Rivet-SDK.md and rivet.go.",
		"",
	}, "\n"))
}

func (s *state) status(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("content-type", "application/json")
	_ = json.NewEncoder(w).Encode(s.snapshot())
}

// ---- the one writable directory -------------------------------------------

func (s *state) writeTicks() {
	path := filepath.Join(s.dataDir, "ticks")
	// Written whole and renamed: a torn count is worse than a lost one, and
	// `/state` is the filesystem an unexpected power cut interrupts.
	tmp := path + ".new"
	if err := os.WriteFile(tmp, []byte(strconv.FormatInt(s.ticks.Load(), 10)), 0o600); err != nil {
		s.j.warn("the tick count could not be written", map[string]any{"why": err.Error()})
		return
	}
	if err := os.Rename(tmp, path); err != nil {
		s.j.warn("the tick count could not be replaced", map[string]any{"why": err.Error()})
	}
}

func readTicks(dir string, j *journal) int64 {
	b, err := os.ReadFile(filepath.Join(dir, "ticks"))
	if err != nil {
		// The ordinary state of a first run, and not a fault.
		return 0
	}
	n, err := strconv.ParseInt(strings.TrimSpace(string(b)), 10, 64)
	if err != nil {
		j.warn("the stored tick count was unreadable and is starting from zero",
			map[string]any{"contents": strings.TrimSpace(string(b))})
		return 0
	}
	return n
}
