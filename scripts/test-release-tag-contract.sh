#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKER="${ROOT}/scripts/check-release-tag-contract.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-tag-contract.XXXXXX")"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "release tag contract test failed: $*" >&2
  exit 1
}

expect_failure() {
  local log_path="$1"
  local expected_message="$2"
  shift 2

  if (
    cd "${RUNNER_REPO}"
    bash "${CHECKER}" "$@"
  ) >"${log_path}" 2>&1; then
    fail "negative fixture unexpectedly passed: $*"
  fi
  if ! grep -F "${expected_message}" "${log_path}" >/dev/null; then
    sed -n '1,120p' "${log_path}" >&2
    fail "negative fixture did not report: ${expected_message}"
  fi
}

write_valid_changelog() {
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' \
    '## [1.0.0] - 2026-07-11' '' '### Fixed' '' \
    '- Release tag contract fixture.' > "${SOURCE_REPO}/CHANGELOG.md"
}

REMOTE_REPO="${TMP_ROOT}/remote.git"
SOURCE_REPO="${TMP_ROOT}/source"
RUNNER_REPO="${TMP_ROOT}/runner"

git init --quiet --bare "${REMOTE_REPO}"
git -C "${REMOTE_REPO}" symbolic-ref HEAD refs/heads/main
git init --quiet --initial-branch=main "${SOURCE_REPO}"
git -C "${SOURCE_REPO}" config user.name "Sigillum release test"
git -C "${SOURCE_REPO}" config user.email "release-test@invalid.example"

printf '%s\n' '[workspace]' 'members = []' '' '[workspace.package]' \
  'version = "1.0.0"' > "${SOURCE_REPO}/Cargo.toml"
write_valid_changelog
git -C "${SOURCE_REPO}" add Cargo.toml CHANGELOG.md
git -C "${SOURCE_REPO}" commit --quiet -m "release fixture"
VALID_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" remote add origin "${REMOTE_REPO}"
git -C "${SOURCE_REPO}" push --quiet --set-upstream origin main

ANNOTATED_TAG="v1.0.0-rc.1"
LIGHTWEIGHT_TAG="v1.0.0-rc.2"
OFF_MAIN_TAG="v1.0.0-rc.3"
EMPTY_CHANGELOG_TAG="v1.0.0-rc.4"
MISSING_CHANGELOG_TAG="v1.0.0-rc.5"
UNDATED_CHANGELOG_TAG="v1.0.0-rc.6"

git -C "${SOURCE_REPO}" tag -a "${ANNOTATED_TAG}" -m "annotated fixture"
git -C "${SOURCE_REPO}" tag "${LIGHTWEIGHT_TAG}"

git -C "${SOURCE_REPO}" switch --quiet -c side
git -C "${SOURCE_REPO}" commit --quiet --allow-empty -m "off-main fixture"
OFF_MAIN_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" tag -a "${OFF_MAIN_TAG}" -m "off-main fixture"
git -C "${SOURCE_REPO}" switch --quiet main

printf '%s\n' '# Changelog' '' '## [Unreleased]' '' \
  '## [1.0.0] - 2026-07-11' '' \
  '[1.0.0]: https://invalid.example/v1.0.0' > "${SOURCE_REPO}/CHANGELOG.md"
git -C "${SOURCE_REPO}" add CHANGELOG.md
git -C "${SOURCE_REPO}" commit --quiet -m "empty changelog fixture"
EMPTY_CHANGELOG_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" tag -a "${EMPTY_CHANGELOG_TAG}" -m "empty changelog fixture"

printf '%s\n' '# Changelog' '' '## [Unreleased]' > "${SOURCE_REPO}/CHANGELOG.md"
git -C "${SOURCE_REPO}" add CHANGELOG.md
git -C "${SOURCE_REPO}" commit --quiet -m "missing changelog fixture"
MISSING_CHANGELOG_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" tag -a "${MISSING_CHANGELOG_TAG}" -m "missing changelog fixture"

printf '%s\n' '# Changelog' '' '## [Unreleased]' '' \
  '## [1.0.0]' '' '### Fixed' '' \
  '- Undated release section fixture.' > "${SOURCE_REPO}/CHANGELOG.md"
git -C "${SOURCE_REPO}" add CHANGELOG.md
git -C "${SOURCE_REPO}" commit --quiet -m "undated changelog fixture"
UNDATED_CHANGELOG_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" tag -a "${UNDATED_CHANGELOG_TAG}" -m "undated changelog fixture"

git -C "${SOURCE_REPO}" push --quiet origin main
git -C "${SOURCE_REPO}" push --quiet origin \
  "${ANNOTATED_TAG}" "${LIGHTWEIGHT_TAG}" "${OFF_MAIN_TAG}" \
  "${EMPTY_CHANGELOG_TAG}" "${MISSING_CHANGELOG_TAG}" \
  "${UNDATED_CHANGELOG_TAG}"

git clone --quiet "${REMOTE_REPO}" "${RUNNER_REPO}"

# Reproduce actions/checkout replacing the local annotated tag ref with the
# peeled commit. The contract must still pass because it reads the remote.
git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
git -C "${RUNNER_REPO}" fetch --quiet --no-tags origin \
  "+${VALID_COMMIT}:refs/tags/${ANNOTATED_TAG}"
[[ "$(git -C "${RUNNER_REPO}" cat-file -t "refs/tags/${ANNOTATED_TAG}")" == "commit" ]] ||
  fail "annotated tag fixture did not collapse to a local commit ref"
REMOTE_TAG_LINES="$(git -C "${RUNNER_REPO}" ls-remote --tags origin \
  "refs/tags/${ANNOTATED_TAG}" "refs/tags/${ANNOTATED_TAG}^{}")"
[[ "$(printf '%s\n' "${REMOTE_TAG_LINES}" | wc -l | tr -d ' ')" == "2" ]] ||
  fail "annotated remote did not advertise direct and peeled refs"
(
  cd "${RUNNER_REPO}"
  bash "${CHECKER}" "${ANNOTATED_TAG}" "${VALID_COMMIT}" origin
)

git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/lightweight.log" "must be annotated" \
  "${LIGHTWEIGHT_TAG}" "${VALID_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${OFF_MAIN_COMMIT}"
expect_failure "${TMP_ROOT}/wrong-sha.log" "expected ${OFF_MAIN_COMMIT}" \
  "${ANNOTATED_TAG}" "${OFF_MAIN_COMMIT}" origin
expect_failure "${TMP_ROOT}/off-main.log" "is not on origin/main history" \
  "${OFF_MAIN_TAG}" "${OFF_MAIN_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${EMPTY_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/empty-changelog.log" "section [1.0.0] is empty" \
  "${EMPTY_CHANGELOG_TAG}" "${EMPTY_CHANGELOG_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${MISSING_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/missing-changelog.log" "needs exactly one dated [1.0.0] section" \
  "${MISSING_CHANGELOG_TAG}" "${MISSING_CHANGELOG_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${UNDATED_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/undated-changelog.log" "needs exactly one dated [1.0.0] section" \
  "${UNDATED_CHANGELOG_TAG}" "${UNDATED_CHANGELOG_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/missing-tag.log" "remote tag was not found" \
  "v1.0.0-rc.99" "${VALID_COMMIT}" origin
expect_failure "${TMP_ROOT}/malformed-tag.log" "release tag syntax is invalid" \
  "v1.0.0-rc.01" "${VALID_COMMIT}" origin

echo "release tag contract tests passed"
