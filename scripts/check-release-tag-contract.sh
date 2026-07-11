#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "release tag contract failed: $*" >&2
  exit 1
}

TAG="${1:-${GITHUB_REF_NAME:-}}"
EXPECTED_SHA="${2:-${GITHUB_SHA:-}}"
REMOTE="${3:-origin}"

[[ -n "${TAG}" ]] || fail "tag is required"
[[ -n "${EXPECTED_SHA}" ]] || fail "expected commit SHA is required"
[[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[1-9][0-9]*)?$ ]] ||
  fail "release tag syntax is invalid: ${TAG}"
[[ "${EXPECTED_SHA}" =~ ^[0-9a-f]{40}$ ]] ||
  fail "expected commit SHA must be a 40-character lowercase hexadecimal object ID"
[[ "${REMOTE}" =~ ^[A-Za-z0-9._-]+$ ]] || fail "remote name is invalid: ${REMOTE}"

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || fail "not inside a git repository"
cd "${ROOT}"

git remote get-url "${REMOTE}" >/dev/null 2>&1 || fail "remote is not configured: ${REMOTE}"

VERSION="$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)"
[[ -n "${VERSION}" ]] || fail "workspace version is missing from Cargo.toml"
VERSION_REGEX="${VERSION//./\\.}"

if [[ "${TAG}" == "v${VERSION}" ]]; then
  :
elif [[ "${TAG}" == "v${VERSION}-rc."* ]]; then
  RC_NUMBER="${TAG#"v${VERSION}-rc."}"
  [[ "${RC_NUMBER}" =~ ^[1-9][0-9]*$ ]] ||
    fail "release tag ${TAG} does not match workspace version ${VERSION}"
else
  fail "release tag ${TAG} does not match workspace version ${VERSION}"
fi

EXPECTED_COMMIT="$(git rev-parse --verify "${EXPECTED_SHA}^{commit}" 2>/dev/null)" ||
  fail "expected SHA does not resolve to a commit: ${EXPECTED_SHA}"
CHECKED_OUT_COMMIT="$(git rev-parse --verify "HEAD^{commit}" 2>/dev/null)" ||
  fail "HEAD does not resolve to a commit"
[[ "${CHECKED_OUT_COMMIT}" == "${EXPECTED_COMMIT}" ]] ||
  fail "checkout HEAD ${CHECKED_OUT_COMMIT} does not match expected commit ${EXPECTED_COMMIT}"

TAG_REF="refs/tags/${TAG}"
PEELED_REF="${TAG_REF}^{}"
if REMOTE_REFS="$(git ls-remote --exit-code --tags "${REMOTE}" "${TAG_REF}" "${PEELED_REF}")"; then
  :
else
  LS_REMOTE_STATUS=$?
  if [[ "${LS_REMOTE_STATUS}" -eq 2 ]]; then
    fail "remote tag was not found: ${TAG_REF}"
  fi
  fail "could not query release tag from remote ${REMOTE}"
fi

REMOTE_TAG_OBJECT=""
REMOTE_PEELED_COMMIT=""
DIRECT_REF_COUNT=0
PEELED_REF_COUNT=0

# Validate the authoritative remote tag. actions/checkout can rewrite the
# runner-local tag ref to the peeled commit during a tag-triggered checkout.
while read -r object_id ref_name; do
  [[ -n "${object_id}" && -n "${ref_name}" ]] || continue
  case "${ref_name}" in
    "${TAG_REF}")
      DIRECT_REF_COUNT=$((DIRECT_REF_COUNT + 1))
      REMOTE_TAG_OBJECT="${object_id}"
      ;;
    "${PEELED_REF}")
      PEELED_REF_COUNT=$((PEELED_REF_COUNT + 1))
      REMOTE_PEELED_COMMIT="${object_id}"
      ;;
    *)
      fail "remote advertised an unexpected ref for ${TAG}: ${ref_name}"
      ;;
  esac
done <<< "${REMOTE_REFS}"

[[ "${DIRECT_REF_COUNT}" -eq 1 ]] || fail "remote must advertise ${TAG_REF} exactly once"
[[ "${PEELED_REF_COUNT}" -le 1 ]] || fail "remote advertised ${PEELED_REF} more than once"
[[ -n "${REMOTE_PEELED_COMMIT}" ]] || fail "release tag ${TAG} must be annotated"
[[ "${#REMOTE_TAG_OBJECT}" -eq 40 && "${REMOTE_TAG_OBJECT}" =~ ^[0-9a-f]+$ ]] ||
  fail "remote tag object is not a valid Git object ID: ${REMOTE_TAG_OBJECT}"
[[ "${#REMOTE_PEELED_COMMIT}" -eq 40 && "${REMOTE_PEELED_COMMIT}" =~ ^[0-9a-f]+$ ]] ||
  fail "remote peeled commit is not a valid Git object ID: ${REMOTE_PEELED_COMMIT}"
[[ "${REMOTE_TAG_OBJECT}" != "${REMOTE_PEELED_COMMIT}" ]] ||
  fail "release tag ${TAG} must resolve through a distinct tag object"
REMOTE_TAG_OBJECT_TYPE="$(git cat-file -t "${REMOTE_TAG_OBJECT}" 2>/dev/null)" ||
  fail "remote tag object is missing from the full-history checkout: ${REMOTE_TAG_OBJECT}"
[[ "${REMOTE_TAG_OBJECT_TYPE}" == "tag" ]] ||
  fail "remote direct ref does not resolve to an annotated tag object"
[[ "${REMOTE_PEELED_COMMIT}" == "${EXPECTED_COMMIT}" ]] ||
  fail "remote tag ${TAG} points to ${REMOTE_PEELED_COMMIT}, expected ${EXPECTED_COMMIT}"

REMOTE_MAIN_REF="refs/remotes/${REMOTE}/main"
git show-ref --verify --quiet "${REMOTE_MAIN_REF}" ||
  fail "remote-tracking main ref is missing: ${REMOTE_MAIN_REF}"
git merge-base --is-ancestor "${REMOTE_PEELED_COMMIT}" "${REMOTE_MAIN_REF}" ||
  fail "release tag ${TAG} is not on ${REMOTE}/main history"

CHANGELOG_HEADER_COUNT="$(
  grep -Ec "^## \\[${VERSION_REGEX}\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md || true
)"
[[ "${CHANGELOG_HEADER_COUNT}" -eq 1 ]] ||
  fail "CHANGELOG.md needs exactly one dated [${VERSION}] section"

awk -v version_regex="${VERSION_REGEX}" '
  $0 ~ "^## \\[" version_regex "\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" {
    in_section = 1
    next
  }
  in_section && $0 ~ "^## \\[" {
    exit
  }
  in_section && $0 !~ /^[[:space:]]*$/ && $0 !~ /^\[[^]]+\]:/ {
    has_content = 1
  }
  END {
    exit has_content ? 0 : 1
  }
' CHANGELOG.md || fail "CHANGELOG.md section [${VERSION}] is empty"

echo "release tag contract passed: ${TAG} -> ${REMOTE_PEELED_COMMIT} on ${REMOTE}/main"
