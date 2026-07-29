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
check_max_lines "crates/sigillum-daemon/src/audit_log.rs" 1800
check_max_lines "crates/sigillum-daemon/src/service/evm.rs" 900
check_max_lines "crates/sigillum-daemon/src/service/evm/rpc.rs" 340
check_max_lines "crates/sigillum-daemon/src/service/evm/rpc/receipt.rs" 120
check_max_lines "crates/sigillum-daemon/src/service/inventory.rs" 750
check_max_lines "crates/sigillum-daemon/src/service/inventory/plan_execution_enqueue.rs" 1400
check_max_lines "crates/sigillum-daemon/src/service/queue.rs" 220
check_max_lines "crates/sigillum-daemon/src/service/queue/outcomes.rs" 180
check_max_lines "crates/sigillum-daemon/src/service/queue/payloads.rs" 140
check_max_lines "crates/sigillum-daemon/src/service/queue/plan_steps.rs" 420
check_max_lines "crates/sigillum-daemon/src/service/queue/plan_steps/receipts.rs" 420
check_max_lines "crates/sigillum-daemon/src/service/queue/plan_steps/signing.rs" 260
check_max_lines "crates/sigillum-daemon/src/service/queue/processing.rs" 400
check_max_lines "crates/sigillum-daemon/src/service/queue/seed_sends.rs" 300
check_max_lines "crates/sigillum-daemon/src/service/queue/serialization.rs" 240
check_max_lines "crates/sigillum-daemon/src/service/queue/state.rs" 320
check_max_lines "crates/sigillum-daemon/src/service/queue/sweeps.rs" 260
check_max_lines "crates/sigillum-daemon/src/service/profiles.rs" 640
check_max_lines "crates/sigillum-daemon/src/service/profiles/resolution.rs" 120
check_max_lines "crates/sigillum-daemon/src/service/profiles/sends.rs" 160
check_max_lines "crates/sigillum-daemon/src/state.rs" 920
check_max_lines "crates/sigillum-daemon/tests/registry_restore_sessions.rs" 1260
check_max_lines "crates/sigillum-daemon/tests/profiles_and_wallets.rs" 1030
check_max_lines "crates/sigillum-daemon/tests/inventory_scan.rs" 2060
check_max_lines "crates/sigillum-daemon/tests/treasury_and_deposits.rs" 1950
check_max_lines "crates/sigillum-api/src/request.rs" 820
check_max_lines "crates/sigillum-api/src/request/queue.rs" 120
check_max_lines "crates/sigillum-api/src/response.rs" 920
check_max_lines "crates/sigillum-api/src/response/queue.rs" 140
check_max_lines "crates/sigillum-api/src/response/queue/plan_step.rs" 160
check_max_lines "crates/sigillum-api/src/response/queue/receipt.rs" 60
check_max_lines "crates/sigillum-cli/src/main.rs" 1450
check_max_lines "crates/sigillum-cli/src/daemon_api.rs" 860
check_max_lines "crates/sigillum-cli/src/daemon_api/plans.rs" 280
check_max_lines "crates/sigillum-cli/src/daemon_api/queue.rs" 80
check_max_lines "crates/sigillum-client/src/lib.rs" 1150
check_max_lines "crates/sigillum-client/src/plans.rs" 160
check_max_lines "crates/sigillum-client/src/queue.rs" 120
check_max_lines "crates/sigillum-daemon/ui/src/app.js" 2500
check_max_lines "crates/sigillum-daemon/ui/src/app.ts" 1500
check_max_lines "crates/sigillum-daemon/ui/src/styles.css" 80
check_max_lines "crates/sigillum-daemon/ui/src/styles/00-design-tokens.css" 120
check_max_lines "crates/sigillum-daemon/ui/src/styles/01-reset-base.css" 90
check_max_lines "crates/sigillum-daemon/ui/src/styles/02-app-shell.css" 150
check_max_lines "crates/sigillum-daemon/ui/src/styles/03-sidebar.css" 310
check_max_lines "crates/sigillum-daemon/ui/src/styles/04-cards-typography.css" 140
check_max_lines "crates/sigillum-daemon/ui/src/styles/05-buttons.css" 160
check_max_lines "crates/sigillum-daemon/ui/src/styles/06-forms.css" 210
check_max_lines "crates/sigillum-daemon/ui/src/styles/07-pills-stats-lists.css" 310
check_max_lines "crates/sigillum-daemon/ui/src/styles/08-states-wallet-toasts.css" 310
check_max_lines "crates/sigillum-daemon/ui/src/styles/09-overview-auth.css" 230
check_max_lines "crates/sigillum-daemon/ui/src/styles/10-setup-wizard.css" 260
check_max_lines "crates/sigillum-daemon/ui/src/styles/11-guide-journey.css" 370
check_max_lines "crates/sigillum-daemon/ui/src/styles/12-modal-utilities-responsive.css" 200
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
check_required_file "crates/sigillum-daemon/src/service/evm/rpc/receipt.rs"
check_required_file "crates/sigillum-daemon/src/service/profiles/resolution.rs"
check_required_file "crates/sigillum-daemon/src/service/profiles/sends.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/outcomes.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/payloads.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/plan_steps.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/plan_steps/receipts.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/plan_steps/signing.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/processing.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/seed_sends.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/serialization.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/state.rs"
check_required_file "crates/sigillum-daemon/src/service/queue/sweeps.rs"
check_required_file "crates/sigillum-daemon/src/service/inventory/plan_execution_enqueue.rs"
check_required_file "crates/sigillum-api/src/request/queue.rs"
check_required_file "crates/sigillum-api/src/response/queue.rs"
check_required_file "crates/sigillum-api/src/response/queue/plan_step.rs"
check_required_file "crates/sigillum-api/src/response/queue/receipt.rs"
check_required_file "crates/sigillum-client/src/queue.rs"
check_required_file "crates/sigillum-client/src/plans.rs"
check_required_file "crates/sigillum-cli/src/daemon_api/queue.rs"
check_required_file "crates/sigillum-cli/src/daemon_api/plans.rs"
check_required_file "crates/sigillum-daemon/ui/src/styles.d.ts"
check_required_file "crates/sigillum-daemon/ui/src/styles/app.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/00-design-tokens.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/01-reset-base.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/02-app-shell.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/03-sidebar.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/04-cards-typography.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/05-buttons.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/06-forms.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/07-pills-stats-lists.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/08-states-wallet-toasts.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/09-overview-auth.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/10-setup-wizard.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/11-guide-journey.css"
check_required_file "crates/sigillum-daemon/ui/src/styles/12-modal-utilities-responsive.css"
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
check_no_inline_tests "crates/sigillum-api/src/response/queue/plan_step.rs"
check_no_inline_tests "crates/sigillum-api/src/response/queue/receipt.rs"
check_no_inline_tests "crates/sigillum-client/src/lib.rs"
check_no_inline_tests "crates/sigillum-client/src/queue.rs"
check_no_inline_tests "crates/sigillum-client/src/plans.rs"

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
check_contains "crates/sigillum-cli/src/daemon_api.rs" '^mod plans;$' "plans CLI API commands must stay in crates/sigillum-cli/src/daemon_api/plans.rs"
check_contains "crates/sigillum-cli/src/daemon_api.rs" '"plans"[[:space:]]*=>[[:space:]]*plans::cmd_api_plans\(args\),' "daemon API plans dispatch must route through the plans module"
check_not_contains "crates/sigillum-cli/src/daemon_api.rs" '^fn cmd_api_plans\(' "plans CLI command handling must not move back into daemon_api.rs"
check_contains "docs/architecture.md" 'refactor-notes\.md' "architecture docs must link the module ownership notes"
check_contains "docs/refactor-notes.md" 'Queue Domain Checkpoint' "refactor notes must record the queue domain checkpoint"

# Lockstep: parity doc Verification route count must match live router registrations.
live_route_count="$(grep -c '\.route(' "${ROOT}/crates/sigillum-daemon/src/routes/mod.rs")"
doc_route_counts="$(
  grep -E 'Route registrations in `crates/sigillum-daemon/src/routes/mod\.rs`: \*\*[0-9]+\*\*' \
    "${ROOT}/docs/operator-surface-parity.md" \
    | grep -oE '\*\*[0-9]+\*\*' \
    | tr -d '*'
)"
doc_route_count_matches="$(printf '%s\n' "${doc_route_counts}" | sed '/^$/d' | wc -l | tr -d ' ')"
if [[ "${doc_route_count_matches}" != "1" ]]; then
  echo "architecture check failed: expected exactly one Verification route-registration count in docs/operator-surface-parity.md; found ${doc_route_count_matches}" >&2
  exit 1
fi
doc_route_count="$(printf '%s\n' "${doc_route_counts}" | sed '/^$/d' | head -n 1)"
if [[ "${live_route_count}" != "${doc_route_count}" ]]; then
  echo "architecture check failed: route registration count drift — live router has ${live_route_count}, docs/operator-surface-parity.md Verification declares ${doc_route_count}. Update the parity doc in the same PR." >&2
  exit 1
fi

echo "architecture checks passed"
