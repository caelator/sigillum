#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

fail() {
  echo "UI accessibility gate failed: $*" >&2
  exit 1
}

command -v node >/dev/null 2>&1 || fail "required command is missing: node"

AXE_SOURCE="crates/sigillum-daemon/ui/node_modules/axe-core/axe.min.js"
if [[ ! -s "${AXE_SOURCE}" ]]; then
  fail "pinned axe-core source is missing; run npm --prefix crates/sigillum-daemon/ui ci --ignore-scripts"
fi

exec node scripts/ui-accessibility.mjs
