#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

GENERATED_ASSETS=(
  "crates/sigillum-daemon/ui/src/app.js"
  "crates/sigillum-daemon/ui/src/styles.css"
)

BEFORE_GENERATED="$(mktemp "${TMPDIR:-/tmp}/sigillum-release-before.XXXXXX")"
AFTER_GENERATED="$(mktemp "${TMPDIR:-/tmp}/sigillum-release-after.XXXXXX")"

cleanup() {
  rm -f "${BEFORE_GENERATED}" "${AFTER_GENERATED}"
}
trap cleanup EXIT

fail() {
  echo "release gate failed: $*" >&2
  exit 1
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

require_cargo_subcommand() {
  local subcommand="$1"
  local install_hint="$2"
  if ! cargo "${subcommand}" --version >/dev/null 2>&1; then
    fail "required cargo subcommand is missing: cargo ${subcommand}. Install with: ${install_hint}"
  fi
}

run_step() {
  echo
  echo "==> $*"
  "$@"
}

run_cargo_metadata() {
  echo
  echo "==> cargo metadata --no-deps --format-version 1"
  cargo metadata --no-deps --format-version 1 >/dev/null
}

run_git_diff_check() {
  echo
  echo "==> git diff --check"
  git diff --check
}

snapshot_generated_assets() {
  : > "${BEFORE_GENERATED}"
  for path in "${GENERATED_ASSETS[@]}"; do
    [[ -f "${path}" ]] || fail "generated asset is missing before build: ${path}"
    shasum -a 256 "${path}" >> "${BEFORE_GENERATED}"
  done
}

verify_generated_assets_unchanged() {
  : > "${AFTER_GENERATED}"
  for path in "${GENERATED_ASSETS[@]}"; do
    [[ -f "${path}" ]] || fail "generated asset is missing after build: ${path}"
    shasum -a 256 "${path}" >> "${AFTER_GENERATED}"
  done

  if ! cmp -s "${BEFORE_GENERATED}" "${AFTER_GENERATED}"; then
    echo "release gate failed: generated daemon UI assets changed during npm build." >&2
    echo "Run npm --prefix crates/sigillum-daemon/ui run build and include the generated assets." >&2
    diff -u "${BEFORE_GENERATED}" "${AFTER_GENERATED}" >&2 || true
    exit 1
  fi
}

require_command cargo
require_command curl
require_command git
require_command node
require_command npm
require_command shasum
require_cargo_subcommand audit "cargo install cargo-audit --version 0.22.1 --locked"
require_cargo_subcommand deny "cargo install cargo-deny --version 0.19.4 --locked"

snapshot_generated_assets

run_cargo_metadata
run_step ./scripts/check-architecture.sh
run_step npm --prefix crates/sigillum-daemon/ui ci --ignore-scripts
run_step npm --prefix crates/sigillum-daemon/ui audit --audit-level=high
run_step npm --prefix crates/sigillum-daemon/ui run typecheck
run_step npm --prefix crates/sigillum-daemon/ui test
run_step npm --prefix crates/sigillum-daemon/ui run build
verify_generated_assets_unchanged
run_step cargo fmt --all --check
run_step cargo check --workspace
run_step cargo test --workspace
run_step ./scripts/check-adversarial.sh
run_step cargo clippy --workspace --all-targets -- -D warnings
run_step ./scripts/check-runtime-smoke.sh
if [[ "${SIGILLUM_SKIP_BROWSER_SMOKE:-0}" == "1" ]]; then
  echo
  echo "==> ./scripts/check-browser-smoke.sh (skipped by SIGILLUM_SKIP_BROWSER_SMOKE=1)"
else
  run_step ./scripts/check-browser-smoke.sh
fi
run_step ./scripts/check-desktop.sh
run_step cargo audit
run_step cargo deny check
run_git_diff_check

echo
echo "release checks passed"
