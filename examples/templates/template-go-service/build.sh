#!/bin/sh
# Build this service for a Rivet — and note what is NOT here.
#
# There is no container and no C toolchain. `CGO_ENABLED=0` makes a Go
# cross-compile two environment variables, so this runs unchanged on Windows,
# macOS and Linux with nothing installed but Go itself. That is the sharpest
# difference between a Go application and a Rust one that compiles C: the Rust
# template needs `tools/container/exec.sh` and `tools/toolchain/env.sh` to reach
# a cross `cc` for SQLite; this needs neither.
#
# Then package it, which DOES need `cargo rivet` and a signing key:
#
#   RIVET_APP_SEED=~/.rivet/keys/rivet-app.seed \
#     cargo rivet package --binary ./template-go-service
#
# `--binary` is what makes a non-Rust program packageable: it takes an
# already-built executable and runs no cargo build at all. The `Cargo.toml`
# beside this file exists only to carry `[package.metadata.rivet]`.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE"

# `-tags timetzdata` is NOT optional: a Rivet has no /usr/share/zoneinfo
# (SPEC-01 §5.2), so time.LoadLocation falls back to UTC in silence without it.
# `-trimpath` keeps this machine's absolute paths out of the binary, which
# AGENTS.md §8.3 forbids in anything tracked and which is good practice besides.
CGO_ENABLED=0 GOOS=linux GOARCH=arm64 \
    go build -trimpath -tags timetzdata -ldflags="-s -w" \
    -o template-go-service .

printf 'built  %s\n' "$HERE/template-go-service"
if command -v file >/dev/null 2>&1; then
    file template-go-service
fi
