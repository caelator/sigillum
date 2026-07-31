#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-}"
RUNNER_KEY="${2:-}"
ARTIFACT_ROOT="${3:-${ROOT}/target}"
SOURCE_DIR="${ARTIFACT_ROOT}/action-artifact-contract"
DOWNLOAD_DIR="${ARTIFACT_ROOT}/action-artifact-download"

fail() {
  echo "artifact action contract failed: $*" >&2
  exit 1
}

[[ -n "${RUNNER_KEY}" ]] || fail "runner key is required"

case "${MODE}" in
  prepare)
    mkdir -p "${SOURCE_DIR}"
    printf '%s\n' "sigillum-action-contract:first:${RUNNER_KEY}" \
      >"${SOURCE_DIR}/first.txt"
    printf '%s\n' "sigillum-action-contract:second:${RUNNER_KEY}" \
      >"${SOURCE_DIR}/second.txt"
    ;;
  verify)
    shopt -s nullglob
    files=("${DOWNLOAD_DIR}"/*)
    [[ "${#files[@]}" -eq 2 ]] ||
      fail "expected exactly 2 downloaded fixture files"
    cmp -s "${SOURCE_DIR}/first.txt" "${DOWNLOAD_DIR}/first.txt" ||
      fail "first artifact fixture differs"
    cmp -s "${SOURCE_DIR}/second.txt" "${DOWNLOAD_DIR}/second.txt" ||
      fail "second artifact fixture differs"
    ;;
  *)
    fail "usage: $0 <prepare|verify> <runner-key> [artifact-root]"
    ;;
esac
