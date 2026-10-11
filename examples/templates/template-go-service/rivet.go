// Package main — **the Rivet contract, in Go.**
//
// This file is the Go answer to `rivet-sdk`, and its existence is the point.
//
// `rivetos-sdk` is a Rust crate; a Go program cannot link it. So everything a
// Rivet hands an application has to be read here independently, from the same
// promises — `ADR-036` §3's table. That makes this file the first real test of
// whether that contract is language-neutral, or whether the Rust crate had
// quietly become the specification.
//
// **Nothing in here is Aura's, and nothing in here is clever.** It is short on
// purpose: if the contract needs more than this to consume, the contract is
// wrong.
//
// # What a Go author must know that a Rust author does not
//
//   - `CGO_ENABLED=0`, so the binary is static and needs no loader and no libc
//     (`ADR-015` §3.1). It also means **no container is needed to build this** —
//     unlike a Rust `-sys` crate, a pure-Go cross-compile is just two variables.
//   - `-tags timetzdata`, because a Rivet has no `/usr/share/zoneinfo`
//     (`SPEC-01` §5.2) and `time.LoadLocation` would otherwise fall back to UTC
//     on a machine whose owner set a timezone. This is silent when it goes
//     wrong, which is why it is in the build recipe and in this comment.
//   - `os.UserConfigDir()` and friends resolve through `$HOME`, which is `/` and
//     read-only. Use [appData] and nothing else.
//   - `os/exec` with a shell fails: there is no shell.
package main

import (
	"encoding/json"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
	"time"
)

// ---------------------------------------------------------------------------
// The handed environment — ADR-036 §3, rows 2 to 4.
// ---------------------------------------------------------------------------

const (
	appDirEnv  = "RIVET_APP_DIR"
	appDataEnv = "RIVET_APP_DATA"
	connectEnv = "RIVET_CONNECT"
)

// appDir is where this application's own installed files are.
//
// The working directory is `/`, which is not it — an application with assets
// can only find them through this. Aura is the case that found it: every font
// failed to open and the renderer segfaulted a second later.
func appDir() (string, error) { return handed(appDirEnv) }

// appData is the one directory this application may write to.
func appData() (string, error) { return handed(appDataEnv) }

// handed reports a missing variable BY NAME rather than defaulting.
//
// A silent fallback would put an application's data somewhere nobody is
// looking, which is the whole reason this refuses instead.
func handed(name string) (string, error) {
	if v, ok := os.LookupEnv(name); ok && v != "" {
		return v, nil
	}
	return "", fmt.Errorf(
		"%s is not set, so this process was not started by a Rivet supervisor "+
			"(see ADR-036 §3 for what a Rivet hands an application)", name)
}

// endpoint is one socket belonging to another application, granted to this one.
type endpoint struct {
	Owner string
	Name  string
	Path  string
}

// endpoints parses RIVET_CONNECT — `owner:name=path`, space separated.
//
// A malformed entry is skipped rather than guessed at: half a path is not a
// better answer than no path.
func endpoints() []endpoint {
	raw := os.Getenv(connectEnv)
	if raw == "" {
		return nil
	}
	var out []endpoint
	for entry := range strings.FieldsSeq(raw) {
		identity, path, ok := strings.Cut(entry, "=")
		if !ok || path == "" {
			continue
		}
		owner, name, ok := strings.Cut(identity, ":")
		if !ok || owner == "" || name == "" {
			continue
		}
		out = append(out, endpoint{Owner: owner, Name: name, Path: path})
	}
	return out
}

// ---------------------------------------------------------------------------
// Listening sockets — ADR-036 §3, row 1.
// ---------------------------------------------------------------------------

const listenFdsStart = 3

// handedSocket is one listening socket the supervisor already bound.
type handedSocket struct {
	Name     string
	Listener net.Listener
}

// listeners adopts every socket the supervisor passed, keyed by its name.
//
// **The application never binds.** `ADR-015` §5.2: Rivet opens the socket and
// passes the descriptor, so an unprivileged uid can serve port 80 without ever
// holding `CAP_NET_BIND_SERVICE`.
//
// `LISTEN_PID` is CHECKED, not assumed. It survives `exec`, so a child this
// process spawned would otherwise believe it owns its parent's descriptors.
//
// Names are `tcp-<port>` for a `listen` entry and the endpoint's own name for a
// `provides` entry — never the manifest's `tcp:8090` spelling, because
// `LISTEN_FDNAMES` is colon-separated and that would arrive as two names.
func listeners() (map[string]net.Listener, error) {
	if pid, ok := os.LookupEnv("LISTEN_PID"); ok {
		claimed, err := strconv.Atoi(strings.TrimSpace(pid))
		if err != nil || claimed != os.Getpid() {
			// Set and not ours. Acting on a descriptor that belongs to a
			// parent is worse than starting with none.
			return nil, nil
		}
	}
	count, err := strconv.Atoi(strings.TrimSpace(os.Getenv("LISTEN_FDS")))
	if err != nil || count <= 0 {
		return nil, nil
	}

	names := strings.Split(os.Getenv("LISTEN_FDNAMES"), ":")
	out := make(map[string]net.Listener, count)
	for i := range count {
		name := ""
		if i < len(names) {
			name = names[i]
		}
		f := os.NewFile(uintptr(listenFdsStart+i), name)
		if f == nil {
			return nil, fmt.Errorf("descriptor %d was not a file", listenFdsStart+i)
		}
		l, err := net.FileListener(f)
		// `net.FileListener` DUPLICATES the descriptor, so the original must be
		// closed or this process holds two of everything for its whole life.
		_ = f.Close()
		if err != nil {
			return nil, fmt.Errorf("descriptor %d (%q) is not a listening socket: %w",
				listenFdsStart+i, name, err)
		}
		out[name] = l
	}
	return out, nil
}

// ---------------------------------------------------------------------------
// The journal — ADR-036 §5.1, ADR-022 §11.
// ---------------------------------------------------------------------------

const (
	ingestSocketPath = "/run/rivet/journal"
	ingestMaxBytes   = 16 * 1024
)

// journal writes structured records the supervisor stamps and stores.
type journal struct {
	svc     string
	boot    string
	started time.Time
}

func newJournal(service string) *journal {
	boot := "unknown"
	if b, err := os.ReadFile("/proc/sys/kernel/random/boot_id"); err == nil {
		boot = strings.TrimSpace(string(b))
	}
	return &journal{svc: service, boot: boot, started: time.Now()}
}

// emit sends one record, and falls back to stderr rather than losing it.
//
// **Every column is emitted even though the journal overwrites five of them.**
// `rivet_core::journal::Record::from_jsonl` REQUIRES boot, mono, sev, svc,
// corr, actor and msg — and a line missing one is not rejected, it becomes a
// `warn` record carrying the raw bytes. An encoder that drifted would not
// fail; it would quietly turn every record into a complaint about itself.
func (j *journal) emit(sev, msg string, fields map[string]any) {
	rec := map[string]any{
		"boot":  j.boot,
		"mono":  float64(int64(time.Since(j.started).Seconds()*1000)) / 1000,
		"sev":   sev,
		"svc":   j.svc,
		"corr":  "",
		"actor": j.svc,
		"msg":   msg,
	}
	for k, v := range fields {
		// A reserved name would put the key on the line twice, which RFC 8259
		// lets a parser resolve either way.
		if _, taken := rec[k]; taken {
			continue
		}
		rec[k] = v
	}

	line, err := json.Marshal(rec)
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s: %s (record would not encode: %v)\n", sev, msg, err)
		return
	}
	line = append(line, '\n')

	if len(line) > ingestMaxBytes || !sendToJournal(line) {
		// stderr is the floor: the supervisor captures it for a process it
		// spawned, so the record is not lost — only less structured.
		os.Stderr.Write(line)
	}
}

func sendToJournal(line []byte) bool {
	c, err := net.DialTimeout("unix", ingestSocketPath, 2*time.Second)
	if err != nil {
		return false
	}
	defer c.Close()
	_, err = c.Write(line)
	return err == nil
}

func (j *journal) info(msg string, f map[string]any)  { j.emit("info", msg, f) }
func (j *journal) warn(msg string, f map[string]any)  { j.emit("warn", msg, f) }
func (j *journal) error(msg string, f map[string]any) { j.emit("error", msg, f) }
