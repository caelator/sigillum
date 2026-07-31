#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKER="${ROOT}/scripts/check-release-state-contract.sh"
CI_WORKFLOW="${ROOT}/.github/workflows/ci.yml"
WORKFLOW="${ROOT}/.github/workflows/release.yml"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-workflow-test.XXXXXX")"
RC_TAG="v1.0.0-rc.6"
FINAL_TAG="v1.0.0"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "release workflow contract test failed: $*" >&2
  exit 1
}

write_release() {
  local path="$1"
  local tag="$2"
  local draft="$3"
  local prerelease="$4"
  local published_at="$5"
  jq -n \
    --arg tag "${tag}" \
    --argjson draft "${draft}" \
    --argjson prerelease "${prerelease}" \
    --argjson published_at "${published_at}" '
      {
        tag_name: $tag,
        draft: $draft,
        prerelease: $prerelease,
        published_at: $published_at
      }
    ' > "${path}"
}

expect_failure() {
  local case_name="$1"
  local expected_message="$2"
  shift 2
  local log_path="${TMP_ROOT}/${case_name}.log"
  if "$@" >"${log_path}" 2>&1; then
    fail "${case_name} unexpectedly passed"
  fi
  grep -F "${expected_message}" "${log_path}" >/dev/null || {
    sed -n '1,120p' "${log_path}" >&2
    fail "${case_name} did not report: ${expected_message}"
  }
}

VALID_RC="${TMP_ROOT}/valid-rc.json"
VALID_FINAL_DRAFT="${TMP_ROOT}/valid-final-draft.json"
VALID_FINAL_PUBLISHED="${TMP_ROOT}/valid-final-published.json"
write_release "${VALID_RC}" "${RC_TAG}" true true null
write_release "${VALID_FINAL_DRAFT}" "${FINAL_TAG}" true false null
write_release \
  "${VALID_FINAL_PUBLISHED}" \
  "${FINAL_TAG}" \
  false \
  false \
  '"2026-07-30T12:00:00Z"'

bash "${CHECKER}" rc-draft "${RC_TAG}" "${VALID_RC}"
bash "${CHECKER}" final-draft "${FINAL_TAG}" "${VALID_FINAL_DRAFT}"
bash "${CHECKER}" final-published "${FINAL_TAG}" "${VALID_FINAL_PUBLISHED}"

RC_NOT_PRERELEASE="${TMP_ROOT}/rc-not-prerelease.json"
write_release "${RC_NOT_PRERELEASE}" "${RC_TAG}" true false null
expect_failure \
  rc-not-prerelease \
  "RC release must be an unpublished prerelease draft" \
  bash "${CHECKER}" rc-draft "${RC_TAG}" "${RC_NOT_PRERELEASE}"

RC_PUBLISHED="${TMP_ROOT}/rc-published.json"
write_release \
  "${RC_PUBLISHED}" \
  "${RC_TAG}" \
  true \
  true \
  '"2026-07-30T12:00:00Z"'
expect_failure \
  rc-published \
  "RC release must be an unpublished prerelease draft" \
  bash "${CHECKER}" rc-draft "${RC_TAG}" "${RC_PUBLISHED}"

RC_NOT_DRAFT="${TMP_ROOT}/rc-not-draft.json"
write_release "${RC_NOT_DRAFT}" "${RC_TAG}" false true null
expect_failure \
  rc-not-draft \
  "RC release must be an unpublished prerelease draft" \
  bash "${CHECKER}" rc-draft "${RC_TAG}" "${RC_NOT_DRAFT}"

FINAL_DRAFT_PRERELEASE="${TMP_ROOT}/final-draft-prerelease.json"
write_release "${FINAL_DRAFT_PRERELEASE}" "${FINAL_TAG}" true true null
expect_failure \
  final-draft-prerelease \
  "final release draft must be unpublished and not a prerelease" \
  bash "${CHECKER}" final-draft "${FINAL_TAG}" "${FINAL_DRAFT_PRERELEASE}"

FINAL_PUBLISHED_PRERELEASE="${TMP_ROOT}/final-published-prerelease.json"
write_release \
  "${FINAL_PUBLISHED_PRERELEASE}" \
  "${FINAL_TAG}" \
  false \
  true \
  '"2026-07-30T12:00:00Z"'
expect_failure \
  final-published-prerelease \
  "final release must be published and not a prerelease" \
  bash "${CHECKER}" final-published "${FINAL_TAG}" \
  "${FINAL_PUBLISHED_PRERELEASE}"

FINAL_UNPUBLISHED="${TMP_ROOT}/final-unpublished.json"
write_release "${FINAL_UNPUBLISHED}" "${FINAL_TAG}" false false null
expect_failure \
  final-unpublished \
  "final release must be published and not a prerelease" \
  bash "${CHECKER}" final-published "${FINAL_TAG}" "${FINAL_UNPUBLISHED}"

for workflow in "${CI_WORKFLOW}" "${WORKFLOW}"; do
  ruby -e '
    require "yaml"
    YAML.safe_load(File.read(ARGV.fetch(0)), aliases: true)
  ' "${workflow}" ||
    fail "workflow is not valid YAML: ${workflow}"
done

ruby -e '
  require "yaml"
  expected = ["ubuntu-24.04", "macos-26"]
  ci = YAML.safe_load(File.read(ARGV.fetch(0)), aliases: true)
  release = YAML.safe_load(File.read(ARGV.fetch(1)), aliases: true)
  abort "CI rust matrix must be exactly ubuntu-24.04 and macos-26" unless
    ci.dig("jobs", "rust", "strategy", "matrix", "os") == expected
  abort "release verify matrix must be exactly ubuntu-24.04 and macos-26" unless
    release.dig("jobs", "verify", "strategy", "matrix", "os") == expected
  abort "release artifacts-macos runner must be macos-26" unless
    release.dig("jobs", "artifacts-macos", "runs-on") == "macos-26"
' "${CI_WORKFLOW}" "${WORKFLOW}" ||
  fail "workflow runner contract is not macOS 26"

for workflow in "${CI_WORKFLOW}" "${WORKFLOW}"; do
  if grep -F 'macos-15' "${workflow}" >/dev/null; then
    fail "workflow still references the retired macOS 15 runner: ${workflow}"
  fi
done

for required_fragment in \
  'release_args+=(--prerelease)' \
  "if [[ \"\${RELEASE_TAG}\" == *-rc.* ]]; then" \
  'bash ./scripts/await-unique-github-release.sh' \
  "\"\${REPOSITORY}\" \"\${RELEASE_TAG}\"" \
  'bash ./scripts/check-release-state-contract.sh' \
  "\"\${release_role}\" \"\${RELEASE_TAG}\"" \
  "rc-draft \"\${PROMOTION_RC_TAG}\""; do
  grep -F "${required_fragment}" "${WORKFLOW}" >/dev/null ||
    fail "release workflow is missing state-contract wiring: ${required_fragment}"
done

if grep -F 'expected exactly one created release' "${WORKFLOW}" >/dev/null; then
  fail "release workflow still contains the immediate post-create list lookup"
fi

echo "release workflow contract tests passed"
