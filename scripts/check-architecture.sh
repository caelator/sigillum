#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

check_max_lines() {
  local path="$1"
  local max_lines="$2"
  local lines
  lines="$(wc -l < "${ROOT}/${path}" | tr -d ' ')"
  if (( lines > max_lines )); then
    echo "architecture check failed: ${path} has ${lines} lines; max is ${max_lines}" >&2
    exit 1
  fi
}

check_max_lines "crates/sigillum-daemon/src/ui.rs" 120
check_max_lines "crates/sigillum-daemon/src/service/inventory.rs" 750
check_max_lines "crates/sigillum-client/src/lib.rs" 1300

if grep -q 'r##"' "${ROOT}/crates/sigillum-daemon/src/ui.rs"; then
  echo "architecture check failed: daemon UI host must not embed raw HTML/JS/CSS strings" >&2
  exit 1
fi

if [[ ! -f "${ROOT}/crates/sigillum-daemon/ui/src/app.js" ]]; then
  echo "architecture check failed: daemon UI runtime script is missing" >&2
  exit 1
fi

if [[ ! -f "${ROOT}/crates/sigillum-daemon/ui/src/api.ts" ]]; then
  echo "architecture check failed: daemon UI typed API module is missing" >&2
  exit 1
fi

echo "architecture checks passed"
