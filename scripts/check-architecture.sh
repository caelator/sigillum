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

check_required_file() {
  local path="$1"
  if [[ ! -f "${ROOT}/${path}" ]]; then
    echo "architecture check failed: required file is missing: ${path}" >&2
    exit 1
  fi
}

check_no_inline_tests() {
  local path="$1"
  if grep -Eq '^[[:space:]]*mod tests[[:space:]]*\{' "${ROOT}/${path}"; then
    echo "architecture check failed: ${path} must keep tests in a separate test module file" >&2
    exit 1
  fi
}

check_max_lines "crates/sigillum-daemon/src/ui.rs" 120
check_max_lines "crates/sigillum-daemon/src/audit_log.rs" 1700
check_max_lines "crates/sigillum-daemon/src/service/evm.rs" 1100
check_max_lines "crates/sigillum-daemon/src/service/inventory.rs" 750
check_max_lines "crates/sigillum-daemon/src/service/queue.rs" 620
check_max_lines "crates/sigillum-daemon/src/service/profiles.rs" 760
check_max_lines "crates/sigillum-daemon/src/state.rs" 920
check_max_lines "crates/sigillum-cli/src/main.rs" 1450
check_max_lines "crates/sigillum-cli/src/daemon_api.rs" 920
check_max_lines "crates/sigillum-client/src/lib.rs" 1300
check_max_lines "crates/sigillum-daemon/ui/src/app.js" 2500
check_max_lines "crates/sigillum-daemon/ui/src/app.ts" 3100
check_max_lines "crates/sigillum-daemon/ui/src/styles.css" 2800

if grep -Eq 'r#+"' "${ROOT}/crates/sigillum-daemon/src/ui.rs"; then
  echo "architecture check failed: daemon UI host must not embed raw HTML/JS/CSS strings" >&2
  exit 1
fi

check_required_file "crates/sigillum-daemon/ui/src/app.js"
check_required_file "crates/sigillum-daemon/ui/src/app.ts"
check_required_file "crates/sigillum-daemon/ui/src/api.ts"
check_required_file "crates/sigillum-daemon/ui/src/api/session.ts"
check_required_file "crates/sigillum-daemon/ui/src/actions/session.ts"
check_required_file "crates/sigillum-daemon/ui/src/render/dom.ts"
check_required_file "crates/sigillum-daemon/ui/src/state/status.ts"
check_required_file "crates/sigillum-daemon/ui/src/state/refresh.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/inventory.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/wallets.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/queue.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/setup.ts"

check_no_inline_tests "crates/sigillum-api/src/request.rs"
check_no_inline_tests "crates/sigillum-api/src/response.rs"
check_no_inline_tests "crates/sigillum-client/src/lib.rs"

echo "architecture checks passed"
