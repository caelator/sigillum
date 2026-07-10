#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

run_step() {
  echo
  echo "==> $*"
  "$@"
}

export PROPTEST_CASES="${SIGILLUM_ADVERSARIAL_PROPTEST_CASES:-256}"

run_step cargo test -p sigillum-core --test fuzz_boundaries --locked
run_step cargo test -p sigillum-daemon --test adversarial_api --locked
run_step cargo test -p sigillum-daemon --test adversarial_execution --locked
run_step cargo test -p sigillum-cli --test cli_smoke --locked
run_step cargo test -p sigillum-gateway --test gateway_tests --locked
run_step cargo test -p sigillum-gateway --test gateway_integration --locked
run_step npm --prefix crates/sigillum-daemon/ui test

echo
echo "adversarial checks passed"
