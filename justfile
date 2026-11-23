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

# Run the tests.
test:
    cargo test --workspace

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

# Drive the console in a browser against a fresh backend.
e2e: console-setup
    cargo build -p famulen
    cd frontend && pnpm exec playwright install --with-deps chromium && pnpm e2e

# Everything CI runs on every OS.
ci: fmt-check lint test stamp-check console-check
