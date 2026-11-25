# Run `nix develop` (or use direnv) first so every tool is on PATH.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List the recipes.
default:
    @just --list

# Format every source.
fmt:
    cargo fmt --all
    nix fmt flake.nix

# Check formatting without changing files.
fmt-check:
    cargo fmt --all --check

# Lint with every warning as an error.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run the tests. The notebooks they make are thrown away at once, so Git does
# not look after each commit whether one needs maintenance. The library path
# Nix's cargo sets on macOS for its own curl makes every program a test starts,
# most of them Git, several times slower; the runner takes it out again.
# Arguments go to cargo.
test *args:
    CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER="env -u DYLD_LIBRARY_PATH" \
    GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=maintenance.auto GIT_CONFIG_VALUE_0=false \
    cargo test --workspace --no-fail-fast {{args}}

# Stamp a copy of the manifests the way a release does, so a placeholder the
# release workflow would no longer find fails here, on every OS.
stamp-check:
    tmp=$(mktemp -d) && cp Cargo.toml Cargo.lock "$tmp" && perl scripts/stamp-version.pl 0.0.0-ci "$tmp"

# Install the console's packages and generate its API client.
console-setup:
    cd frontend && pnpm install --frozen-lockfile && pnpm gen

# Type check the console and build it the way its container does.
console-check: console-setup
    cd frontend && pnpm check && pnpm build

# Run the API on :7790 and the console on http://127.0.0.1:3000 together.
dev: console-setup
    #!/usr/bin/env bash
    set -euo pipefail
    FAMULEN_LISTEN=127.0.0.1:7790 cargo run -p famulen &
    trap 'kill %1' EXIT
    cd frontend && FAMULEN_API_URL=http://127.0.0.1:7790 pnpm exec vite dev --port 3000 --host 127.0.0.1

# Drive the console in a browser against a fresh backend. Arguments go to
# Playwright.
e2e *args: console-setup
    cargo build -p famulen
    cd frontend && pnpm exec playwright install --with-deps chromium && pnpm e2e {{args}}

# Everything CI runs on Linux.
ci: fmt-check lint test stamp-check console-check

# What CI runs on macOS: the tests an OS can make a difference to. The rest of
# `just ci` is the same on every OS, and Linux checks it.
ci-macos:
    skip=$(perl scripts/os-free-tests.pl) && just test --tests -- $skip
