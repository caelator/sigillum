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

check_contains() {
  local path="$1"
  local pattern="$2"
  local message="$3"
  if ! grep -Eq "${pattern}" "${ROOT}/${path}"; then
    echo "architecture check failed: ${message}" >&2
    exit 1
  fi
}

check_not_contains() {
  local path="$1"
  local pattern="$2"
  local message="$3"
  if grep -Eq "${pattern}" "${ROOT}/${path}"; then
    echo "architecture check failed: ${message}" >&2
    exit 1
  fi
}

check_max_lines "crates/sigillum-daemon/src/ui.rs" 120
check_max_lines "crates/sigillum-daemon/src/audit_log.rs" 1700
check_max_lines "crates/sigillum-daemon/src/service/evm.rs" 900
check_max_lines "crates/sigillum-daemon/src/service/evm/rpc.rs" 340
check_max_lines "crates/sigillum-daemon/src/service/inventory.rs" 750
check_max_lines "crates/sigillum-daemon/src/service/queue.rs" 220
check_max_lines "crates/sigillum-daemon/src/service/queue/payloads.rs" 140
check_max_lines "crates/sigillum-daemon/src/service/queue/processing.rs" 320
check_max_lines "crates/sigillum-daemon/src/service/queue/state.rs" 320
check_max_lines "crates/sigillum-daemon/src/service/queue/sweeps.rs" 260
check_max_lines "crates/sigillum-daemon/src/service/profiles.rs" 640
check_max_lines "crates/sigillum-daemon/src/service/profiles/resolution.rs" 120
check_max_lines "crates/sigillum-daemon/src/service/profiles/sends.rs" 160
check_max_lines "crates/sigillum-daemon/src/state.rs" 920
check_max_lines "crates/sigillum-api/src/request.rs" 820
check_max_lines "crates/sigillum-api/src/request/queue.rs" 120
check_max_lines "crates/sigillum-api/src/response.rs" 920
check_max_lines "crates/sigillum-api/src/response/queue.rs" 140
check_max_lines "crates/sigillum-cli/src/main.rs" 1450
check_max_lines "crates/sigillum-cli/src/daemon_api.rs" 860
check_max_lines "crates/sigillum-cli/src/daemon_api/queue.rs" 80
check_max_lines "crates/sigillum-client/src/lib.rs" 1150
check_max_lines "crates/sigillum-client/src/queue.rs" 120
check_max_lines "crates/sigillum-daemon/ui/src/app.js" 2500
check_max_lines "crates/sigillum-daemon/ui/src/app.ts" 1500
check_max_lines "crates/sigillum-daemon/ui/src/styles.css" 80
check_max_lines "crates/sigillum-daemon/ui/src/styles/00-foundation-tokens.css" 120
check_max_lines "crates/sigillum-daemon/ui/src/styles/01-foundation-base-layout.css" 220
check_max_lines "crates/sigillum-daemon/ui/src/styles/02-foundation-forms.css" 120
check_max_lines "crates/sigillum-daemon/ui/src/styles/03-foundation-components.css" 450
check_max_lines "crates/sigillum-daemon/ui/src/styles/10-refresh-workspace.css" 500
check_max_lines "crates/sigillum-daemon/ui/src/styles/11-refresh-forms.css" 120
check_max_lines "crates/sigillum-daemon/ui/src/styles/12-refresh-components.css" 260
check_max_lines "crates/sigillum-daemon/ui/src/styles/13-refresh-responsive.css" 120
check_max_lines "crates/sigillum-daemon/ui/src/styles/20-console-workspace.css" 650
check_max_lines "crates/sigillum-daemon/ui/src/styles/21-console-forms.css" 260
check_max_lines "crates/sigillum-daemon/ui/src/styles/22-console-components.css" 260
check_max_lines "crates/sigillum-daemon/ui/src/styles/23-console-responsive.css" 160
check_max_lines "crates/sigillum-daemon/ui/src/styles/30-final-polish.css" 650
check_max_lines "crates/sigillum-daemon/ui/src/styles/app.css" 80
check_max_lines "docs/refactor-notes.md" 220

if grep -Eq 'r#+"' "${ROOT}/crates/sigillum-daemon/src/ui.rs"; then
  echo "architecture check failed: daemon UI host must not embed raw HTML/JS/CSS strings" >&2
  exit 1
fi

check_required_file "crates/sigillum-daemon/ui/src/app.js"
check_required_file "crates/sigillum-daemon/ui/src/app.ts"
check_required_file "crates/sigillum-daemon/ui/src/api.ts"
check_required_file "crates/sigillum-daemon/src/service/evm/rpc.rs"
check_required_file "crates/sigillum-daemon/src/service/profiles/resolution.rs"
check_required_file "crates/sigillum-daemon/src/service/profiles/sends.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/payloads.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/processing.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/state.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/sweeps.rs"
check_required_file "crates/sigillum-api/src/request/queue.rs"
check_required_file "crates/sigillum-api/src/response/queue.rs"
check_required_file "crates/sigillum-client/src/queue.rs"
check_required_file "crates/sigillum-cli/src/daemon_api/queue.rs"
check_required_file "crates/sigillum-daemon/ui/src/styles.d.ts"
check_required_file "crates/sigillum-daemon/ui/src/styles/app.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/00-foundation-tokens.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/01-foundation-base-layout.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/02-foundation-forms.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/03-foundation-components.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/10-refresh-workspace.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/11-refresh-forms.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/12-refresh-components.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/13-refresh-responsive.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/20-console-workspace.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/21-console-forms.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/22-console-components.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/23-console-responsive.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/30-final-polish.css"
check_required_file "crates/sigillum-daemon/ui/src/api/session.ts"
check_required_file "crates/sigillum-daemon/ui/src/actions/session.ts"
check_required_file "crates/sigillum-daemon/ui/src/actions/dispatcher.ts"
check_required_file "crates/sigillum-daemon/ui/src/render/dom.ts"
check_required_file "crates/sigillum-daemon/ui/src/render/forms.ts"
check_required_file "crates/sigillum-daemon/ui/src/render/html.ts"
check_required_file "crates/sigillum-daemon/ui/src/state/status.ts"
check_required_file "crates/sigillum-daemon/ui/src/state/refresh.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/inventory.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/wallets.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/queue.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/fido2.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/operations.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/shell.ts"
check_required_file "crates/sigillum-daemon/ui/src/views/setup.ts"
check_required_file "crates/sigillum-daemon/ui/test/ui-smoke.test.ts"
check_required_file "docs/refactor-notes.md"

check_no_inline_tests "crates/sigillum-api/src/request.rs"
check_no_inline_tests "crates/sigillum-api/src/request/queue.rs"
check_no_inline_tests "crates/sigillum-api/src/response.rs"
check_no_inline_tests "crates/sigillum-api/src/response/queue.rs"
check_no_inline_tests "crates/sigillum-client/src/lib.rs"
check_no_inline_tests "crates/sigillum-client/src/queue.rs"

check_contains "crates/sigillum-api/src/request.rs" '^mod queue;$' "queue request contracts must stay in crates/sigillum-api/src/request/queue.rs"
check_contains "crates/sigillum-api/src/request.rs" '^pub use queue::\*;$' "queue request contract names must remain re-exported from request.rs"
check_not_contains "crates/sigillum-api/src/request.rs" '^(pub struct|pub type) Queue' "queue request DTOs must not move back into request.rs"
check_contains "crates/sigillum-api/src/response.rs" '^mod queue;$' "queue response contracts must stay in crates/sigillum-api/src/response/queue.rs"
check_contains "crates/sigillum-api/src/response.rs" '^pub use queue::\*;$' "queue response contract names must remain re-exported from response.rs"
check_not_contains "crates/sigillum-api/src/response.rs" '^(pub struct|pub enum) Queue' "queue response DTOs must not move back into response.rs"
check_contains "crates/sigillum-client/src/lib.rs" '^mod queue;$' "queue client methods must stay in crates/sigillum-client/src/queue.rs"
check_not_contains "crates/sigillum-client/src/lib.rs" 'pub async fn (list_queue_jobs|enqueue_eth_stealth_transfer|enqueue_eth_stealth_erc20_transfer|enqueue_eth_stealth_native_sweep|enqueue_eth_stealth_erc20_sweep|process_queue)' "queue client methods must not move back into sigillum-client/src/lib.rs"
check_contains "crates/sigillum-cli/src/daemon_api.rs" '^mod queue;$' "queue CLI API commands must stay in crates/sigillum-cli/src/daemon_api/queue.rs"
check_contains "crates/sigillum-cli/src/daemon_api.rs" '"queue"[[:space:]]*=>[[:space:]]*queue::cmd_api_queue\(args\),' "daemon API queue dispatch must route through the queue module"
check_not_contains "crates/sigillum-cli/src/daemon_api.rs" '^fn cmd_api_queue\(' "queue CLI command handling must not move back into daemon_api.rs"
check_contains "docs/architecture.md" 'refactor-notes\.md' "architecture docs must link the module ownership notes"
check_contains "docs/refactor-notes.md" 'Queue Domain Checkpoint' "refactor notes must record the queue domain checkpoint"

echo "architecture checks passed"
