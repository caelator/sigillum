#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
SNAPSHOT_PATH="${2:-}"

if [[ -z "${MODE}" || -z "${SNAPSHOT_PATH}" ]]; then
  echo "usage: $0 <snapshot|verify> <snapshot-path>" >&2
  exit 2
fi

snapshot_tracked_diff() {
  git diff --binary --no-ext-diff HEAD -- > "${SNAPSHOT_PATH}"
}

case "${MODE}" in
  snapshot)
    snapshot_tracked_diff
    ;;
  verify)
    AFTER_PATH="$(mktemp "${TMPDIR:-/tmp}/sigillum-release-tracked-after.XXXXXX")"
    trap 'rm -f "${AFTER_PATH}"' EXIT
    git diff --binary --no-ext-diff HEAD -- > "${AFTER_PATH}"
    if ! cmp -s "${SNAPSHOT_PATH}" "${AFTER_PATH}"; then
      echo "release gate failed: a check mutated tracked files" >&2
      echo "Current tracked-file status (contents suppressed):" >&2
      git status --short --untracked-files=no >&2
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 <snapshot|verify> <snapshot-path>" >&2
    exit 2
    ;;
esac
