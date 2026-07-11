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

expect_failure_in_repo() {
  local repo_path="$1"
  local log_path="$2"
  local expected_message="$3"
  shift 3

  if (
    cd "${repo_path}"
    bash "${CHECKER}" "$@"
  ) >"${log_path}" 2>&1; then
    fail "negative fixture unexpectedly passed: $*"
  fi
  if ! grep -F "${expected_message}" "${log_path}" >/dev/null; then
    sed -n '1,120p' "${log_path}" >&2
    fail "negative fixture did not report: ${expected_message}"
  fi
}

expect_failure() {
  expect_failure_in_repo "${RUNNER_REPO}" "$@"
}

write_valid_changelog() {
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' \
    '## [1.0.0] - 2026-07-11' '' '### Fixed' '' \
    '- Release tag contract fixture.' > "${SOURCE_REPO}/CHANGELOG.md"
}

REMOTE_REPO="${TMP_ROOT}/remote.git"
SOURCE_REPO="${TMP_ROOT}/source"
RUNNER_REPO="${TMP_ROOT}/runner"
OBJECT_MISSING_REPO="${TMP_ROOT}/object-missing-runner"
NO_RC_RUNNER_REPO="${TMP_ROOT}/no-rc-runner"
LEGACY_REMOTE_REPO="${TMP_ROOT}/legacy-remote.git"
LEGACY_RUNNER_REPO="${TMP_ROOT}/legacy-runner"

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
SKIPPED_RC_TAG="v1.0.0-rc.8"
MISSING_SEQUENCE_TAG="v1.0.0-rc.7"
HIGHEST_LIGHTWEIGHT_TAG="v1.0.0-rc.9"
FINAL_TAG="v1.0.0"

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

git -C "${SOURCE_REPO}" tag -a "${SKIPPED_RC_TAG}" \
  -m "skipped RC fixture" "${VALID_COMMIT}"
git -C "${SOURCE_REPO}" tag -a "${MISSING_SEQUENCE_TAG}" \
  -m "filled sequence fixture" "${VALID_COMMIT}"
git -C "${SOURCE_REPO}" tag -a "${FINAL_TAG}" \
  -m "final release fixture" "${VALID_COMMIT}"

# The real 1.0.0 history burned and deleted rc.1 under the superseded policy.
# Prove that the one documented legacy sequence starting at annotated rc.2 is
# accepted, then restore rc.2 as the lightweight negative fixture below.
git init --quiet --bare "${LEGACY_REMOTE_REPO}"
git -C "${LEGACY_REMOTE_REPO}" symbolic-ref HEAD refs/heads/main
git -C "${SOURCE_REPO}" remote add legacy "${LEGACY_REMOTE_REPO}"
git -C "${SOURCE_REPO}" push --quiet legacy main
git -C "${SOURCE_REPO}" tag --force -a "${LIGHTWEIGHT_TAG}" \
  -m "legacy retained rc.2 fixture" "${VALID_COMMIT}"
git -C "${SOURCE_REPO}" push --quiet legacy "${LIGHTWEIGHT_TAG}"
git clone --quiet "${LEGACY_REMOTE_REPO}" "${LEGACY_RUNNER_REPO}"
git -C "${LEGACY_RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
(
  cd "${LEGACY_RUNNER_REPO}"
  bash "${CHECKER}" "${LIGHTWEIGHT_TAG}" "${VALID_COMMIT}" origin
)
git -C "${SOURCE_REPO}" tag --force "${LIGHTWEIGHT_TAG}" "${VALID_COMMIT}"

ANNOTATED_TAG_OBJECT="$(git -C "${SOURCE_REPO}" rev-parse "refs/tags/${ANNOTATED_TAG}")"

git -C "${SOURCE_REPO}" push --quiet origin main

# A final release is invalid without a retained RC anchor, even when the final
# tag itself is annotated and points to main.
git -C "${SOURCE_REPO}" push --quiet origin "${FINAL_TAG}"
git clone --quiet "${REMOTE_REPO}" "${NO_RC_RUNNER_REPO}"
git -C "${NO_RC_RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure_in_repo "${NO_RC_RUNNER_REPO}" "${TMP_ROOT}/final-no-rc.log" \
  "requires at least one retained RC tag" \
  "${FINAL_TAG}" "${VALID_COMMIT}" origin
git -C "${SOURCE_REPO}" push --quiet origin ":refs/tags/${FINAL_TAG}"

git -C "${SOURCE_REPO}" push --quiet origin "${ANNOTATED_TAG}"

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
  GITHUB_OUTPUT="${TMP_ROOT}/github-output" \
    bash "${CHECKER}" "${ANNOTATED_TAG}" "${VALID_COMMIT}" origin \
      "${ANNOTATED_TAG_OBJECT}"
)
grep -Fx "tag_object=${ANNOTATED_TAG_OBJECT}" "${TMP_ROOT}/github-output" >/dev/null ||
  fail "contract did not publish the authoritative tag object output"

expect_failure "${TMP_ROOT}/wrong-tag-object.log" "does not match expected ${VALID_COMMIT}" \
  "${ANNOTATED_TAG}" "${VALID_COMMIT}" origin "${VALID_COMMIT}"

# A no-tags checkout has the peeled commit and origin/main, but no annotated
# tag object. The checker must fetch the authoritative tag into a scratch ref,
# never recreate or mutate refs/tags/<tag>.
git clone --quiet --no-local --no-tags "${REMOTE_REPO}" "${OBJECT_MISSING_REPO}"
git -C "${OBJECT_MISSING_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
if git -C "${OBJECT_MISSING_REPO}" cat-file -e "${ANNOTATED_TAG_OBJECT}^{object}" 2>/dev/null; then
  fail "no-tags fixture unexpectedly contains the annotated tag object"
fi
(
  cd "${OBJECT_MISSING_REPO}"
  bash "${CHECKER}" "${ANNOTATED_TAG}" "${VALID_COMMIT}" origin \
    "${ANNOTATED_TAG_OBJECT}"
)
[[ "$(git -C "${OBJECT_MISSING_REPO}" cat-file -t "${ANNOTATED_TAG_OBJECT}")" == "tag" ]] ||
  fail "object-missing fixture did not fetch the authoritative tag object"
if [[ -n "$(git -C "${OBJECT_MISSING_REPO}" for-each-ref \
  --format='%(refname)' refs/sigillum/release-contract)" ]]; then
  fail "object-missing fixture retained a temporary scratch ref"
fi
if git -C "${OBJECT_MISSING_REPO}" show-ref --verify --quiet \
  "refs/tags/${ANNOTATED_TAG}"; then
  fail "object-missing fixture recreated a runner-local tag ref"
fi

# Simulate a same-name/same-commit retag between the contract and release jobs.
# The pinned first-job tag-object ID must make the later check fail closed.
git -C "${SOURCE_REPO}" tag --force -a "${ANNOTATED_TAG}" \
  -m "replacement annotated fixture" "${VALID_COMMIT}"
REPLACEMENT_TAG_OBJECT="$(git -C "${SOURCE_REPO}" rev-parse "refs/tags/${ANNOTATED_TAG}")"
[[ "${REPLACEMENT_TAG_OBJECT}" != "${ANNOTATED_TAG_OBJECT}" ]] ||
  fail "replacement tag fixture did not create a distinct tag object"
git -C "${SOURCE_REPO}" push --quiet --force origin "${ANNOTATED_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/retagged-object.log" \
  "does not match expected ${ANNOTATED_TAG_OBJECT}" \
  "${ANNOTATED_TAG}" "${VALID_COMMIT}" origin "${ANNOTATED_TAG_OBJECT}"

git -C "${SOURCE_REPO}" push --quiet origin "${LIGHTWEIGHT_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/lightweight.log" "must be annotated" \
  "${LIGHTWEIGHT_TAG}" "${VALID_COMMIT}" origin

git -C "${SOURCE_REPO}" push --quiet origin "${OFF_MAIN_TAG}"
git -C "${RUNNER_REPO}" fetch --quiet --no-tags origin \
  "refs/tags/${OFF_MAIN_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${OFF_MAIN_COMMIT}"
expect_failure "${TMP_ROOT}/wrong-sha.log" "expected ${OFF_MAIN_COMMIT}" \
  "${ANNOTATED_TAG}" "${OFF_MAIN_COMMIT}" origin
expect_failure "${TMP_ROOT}/off-main.log" "is not on origin/main history" \
  "${OFF_MAIN_TAG}" "${OFF_MAIN_COMMIT}" origin

git -C "${SOURCE_REPO}" push --quiet origin "${EMPTY_CHANGELOG_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${EMPTY_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/empty-changelog.log" "section [1.0.0] is empty" \
  "${EMPTY_CHANGELOG_TAG}" "${EMPTY_CHANGELOG_COMMIT}" origin

git -C "${SOURCE_REPO}" push --quiet origin "${MISSING_CHANGELOG_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${MISSING_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/missing-changelog.log" "needs exactly one dated [1.0.0] section" \
  "${MISSING_CHANGELOG_TAG}" "${MISSING_CHANGELOG_COMMIT}" origin

git -C "${SOURCE_REPO}" push --quiet origin "${UNDATED_CHANGELOG_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${UNDATED_CHANGELOG_COMMIT}"
expect_failure "${TMP_ROOT}/undated-changelog.log" "needs exactly one dated [1.0.0] section" \
  "${UNDATED_CHANGELOG_TAG}" "${UNDATED_CHANGELOG_COMMIT}" origin

git -C "${SOURCE_REPO}" push --quiet origin "${SKIPPED_RC_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/skipped-rc.log" "expected rc.7" \
  "${SKIPPED_RC_TAG}" "${VALID_COMMIT}" origin

# Fill the intentionally skipped fixture so final-release tests can exercise a
# complete retained sequence. The skipped-number check above proves rc.8 could
# not have been accepted at the time it was introduced.
git -C "${SOURCE_REPO}" push --quiet origin "${MISSING_SEQUENCE_TAG}"

git -C "${SOURCE_REPO}" push --quiet origin "${FINAL_TAG}"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
(
  cd "${RUNNER_REPO}"
  bash "${CHECKER}" "${FINAL_TAG}" "${VALID_COMMIT}" origin
)

write_valid_changelog
git -C "${SOURCE_REPO}" add CHANGELOG.md
git -C "${SOURCE_REPO}" commit --quiet -m "final descendant fixture"
FINAL_DESCENDANT_COMMIT="$(git -C "${SOURCE_REPO}" rev-parse HEAD)"
git -C "${SOURCE_REPO}" tag --force -a "${FINAL_TAG}" \
  -m "descendant final fixture" "${FINAL_DESCENDANT_COMMIT}"
git -C "${SOURCE_REPO}" push --quiet origin main
git -C "${SOURCE_REPO}" push --quiet --force origin "${FINAL_TAG}"
git -C "${RUNNER_REPO}" fetch --quiet origin \
  "+refs/heads/main:refs/remotes/origin/main"
git -C "${RUNNER_REPO}" checkout --quiet --detach "${FINAL_DESCENDANT_COMMIT}"
expect_failure "${TMP_ROOT}/final-descendant.log" "must use the same commit" \
  "${FINAL_TAG}" "${FINAL_DESCENDANT_COMMIT}" origin

git -C "${SOURCE_REPO}" tag "${HIGHEST_LIGHTWEIGHT_TAG}" \
  "${FINAL_DESCENDANT_COMMIT}"
git -C "${SOURCE_REPO}" push --quiet origin "${HIGHEST_LIGHTWEIGHT_TAG}"
expect_failure "${TMP_ROOT}/final-lightweight-rc.log" "must be annotated" \
  "${FINAL_TAG}" "${FINAL_DESCENDANT_COMMIT}" origin

git -C "${RUNNER_REPO}" checkout --quiet --detach "${VALID_COMMIT}"
expect_failure "${TMP_ROOT}/missing-tag.log" "remote tag was not found" \
  "v1.0.0-rc.99" "${VALID_COMMIT}" origin
expect_failure "${TMP_ROOT}/malformed-tag.log" "release tag syntax is invalid" \
  "v1.0.0-rc.01" "${VALID_COMMIT}" origin

echo "release tag contract tests passed"
