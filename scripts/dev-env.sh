#!/usr/bin/env bash
# Toolchain/cache environment for this repo's read-only-root WSL2 dev box.
#
# Why: $HOME is read-only in this environment, so rustup/cargo/pnpm/npm must
# never try to write their caches there. This script redirects all of that
# into git-ignored, repo-relative directories (.rustup/, .cargo/, .pnpm/,
# .cache/, ...) so a normal `cargo`/`pnpm` invocation just works.
#
# Usage: source this in every new shell before running cargo or pnpm:
#   source scripts/dev-env.sh
#
# This script does NOT provision the toolchain or the cargo registry cache —
# it only points standard tools at repo-local directories. On a fresh clone,
# .rustup/ and .cargo/ are empty (they're git-ignored) and must be populated
# once, e.g. via `rustup-init` targeting RUSTUP_HOME/CARGO_HOME below, plus
# `rustup target add x86_64-pc-windows-msvc` for the Windows cross-check.

SP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

export RUSTUP_HOME="$SP_ROOT/.rustup"
export CARGO_HOME="$SP_ROOT/.cargo"

# `.tools` carries a stand-in `llvm-rc` used by `cargo check --target
# x86_64-pc-windows-msvc` (see .cargo/config.toml for how it's wired in via
# the RC_<target> env var embed-resource reads — this PATH entry is a
# fallback for any tool that shells out to a bare `llvm-rc`).
export PATH="$CARGO_HOME/bin:$SP_ROOT/.tools:$SP_ROOT/.pnpm:$PATH"

# Redirect any tool that tries to write under $HOME.
export XDG_CACHE_HOME="$SP_ROOT/.cache"
export XDG_CONFIG_HOME="$SP_ROOT/.config"
export XDG_DATA_HOME="$SP_ROOT/.local/share"
export XDG_STATE_HOME="$SP_ROOT/.local/state"
export NPM_CONFIG_CACHE="$SP_ROOT/.npm"
export NPM_CONFIG_USERCONFIG="$SP_ROOT/.npmrc"
export PNPM_HOME="$SP_ROOT/.pnpm"
export COREPACK_HOME="$SP_ROOT/.corepack"

export RUST_BACKTRACE=1
