#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="${ROOT}/scripts/release-tracked-state.sh"
TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-tree-test.XXXXXX")"
trap 'rm -rf "${TEMP_ROOT}"' EXIT

REPO="${TEMP_ROOT}/repo"
SNAPSHOT="${TEMP_ROOT}/snapshot.diff"
mkdir -p "${REPO}"
git -C "${REPO}" init -q
git -C "${REPO}" config user.email "release-test@sigillum.invalid"
git -C "${REPO}" config user.name "Sigillum release test"
printf 'initial\n' > "${REPO}/tracked.txt"
git -C "${REPO}" add tracked.txt
git -C "${REPO}" commit -qm "initial"

(
  cd "${REPO}"
  bash "${HELPER}" snapshot "${SNAPSHOT}"
  bash "${HELPER}" verify "${SNAPSHOT}"

  printf 'untracked\n' > untracked.txt
  bash "${HELPER}" verify "${SNAPSHOT}"

  printf 'mutated\n' > tracked.txt
  if bash "${HELPER}" verify "${SNAPSHOT}" >/dev/null 2>&1; then
    echo "tracked mutation was not detected" >&2
    exit 1
  fi

  git restore tracked.txt
  printf 'pre-existing change\n' > tracked.txt
  bash "${HELPER}" snapshot "${SNAPSHOT}"
  bash "${HELPER}" verify "${SNAPSHOT}"

  printf 'second mutation\n' >> tracked.txt
  if bash "${HELPER}" verify "${SNAPSHOT}" >/dev/null 2>&1; then
    echo "mutation after a dirty snapshot was not detected" >&2
    exit 1
  fi
)

echo "release tracked-state tests passed"
