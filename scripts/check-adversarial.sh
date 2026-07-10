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

run_step cargo test -p sigillum-core --test fuzz_boundaries
run_step cargo test -p sigillum-daemon --test adversarial_api
run_step cargo test -p sigillum-daemon --test adversarial_execution
run_step cargo test -p sigillum-cli --test cli_smoke
run_step cargo test -p sigillum-gateway --test gateway_tests
run_step cargo test -p sigillum-gateway --test gateway_integration
run_step npm --prefix crates/sigillum-daemon/ui test

echo
echo "adversarial checks passed"
