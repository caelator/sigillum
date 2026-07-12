#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "release tag contract failed: $*" >&2
  exit 1
}

SCRATCH_REF=""
cleanup_scratch_ref() {
  if [[ -n "${SCRATCH_REF}" ]]; then
    git update-ref -d "${SCRATCH_REF}" >/dev/null 2>&1 || true
  fi
}
trap cleanup_scratch_ref EXIT

TAG="${1:-${GITHUB_REF_NAME:-}}"
EXPECTED_SHA="${2:-${GITHUB_SHA:-}}"
REMOTE="${3:-origin}"
EXPECTED_TAG_OBJECT="${4:-}"

[[ -n "${TAG}" ]] || fail "tag is required"
[[ -n "${EXPECTED_SHA}" ]] || fail "expected commit SHA is required"
[[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[1-9][0-9]*)?$ ]] ||
  fail "release tag syntax is invalid: ${TAG}"
[[ "${EXPECTED_SHA}" =~ ^[0-9a-f]{40}$ ]] ||
  fail "expected commit SHA must be a 40-character lowercase hexadecimal object ID"
if [[ -n "${EXPECTED_TAG_OBJECT}" ]]; then
  [[ "${EXPECTED_TAG_OBJECT}" =~ ^[0-9a-f]{40}$ ]] ||
    fail "expected tag object must be a 40-character lowercase hexadecimal object ID"
fi
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
  [[ "${RC_NUMBER}" =~ ^[1-9][0-9]*$ && "${#RC_NUMBER}" -le 9 ]] ||
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

# An explicit full-history checkout normally has the tag object even if the
# tag ref was rewritten to the peeled commit. Fetch the authoritative remote
# ref into a non-tag scratch ref when the object itself is absent, then prove
# that the fetched object still matches the earlier ls-remote observation.
if ! git cat-file -e "${REMOTE_TAG_OBJECT}^{object}" 2>/dev/null; then
  SCRATCH_REF="refs/sigillum/release-contract/${REMOTE_TAG_OBJECT}-$$"
  git fetch --quiet --no-tags --force "${REMOTE}" \
    "+${TAG_REF}:${SCRATCH_REF}" ||
    fail "could not fetch remote tag object into scratch ref: ${TAG_REF}"
  FETCHED_TAG_OBJECT="$(git rev-parse --verify "${SCRATCH_REF}^{object}" 2>/dev/null)" ||
    fail "scratch ref does not resolve to a Git object: ${SCRATCH_REF}"
  [[ "${FETCHED_TAG_OBJECT}" == "${REMOTE_TAG_OBJECT}" ]] ||
    fail "remote tag ${TAG} changed while validating its tag object"
  git update-ref -d "${SCRATCH_REF}" "${REMOTE_TAG_OBJECT}" ||
    fail "could not remove temporary release-contract scratch ref"
  SCRATCH_REF=""
fi

REMOTE_TAG_OBJECT_TYPE="$(git cat-file -t "${REMOTE_TAG_OBJECT}" 2>/dev/null)" ||
  fail "remote tag object is unavailable after the scratch fetch: ${REMOTE_TAG_OBJECT}"
[[ "${REMOTE_TAG_OBJECT_TYPE}" == "tag" ]] ||
  fail "remote direct ref does not resolve to an annotated tag object"
if [[ -n "${EXPECTED_TAG_OBJECT}" ]]; then
  [[ "${REMOTE_TAG_OBJECT}" == "${EXPECTED_TAG_OBJECT}" ]] ||
    fail "remote tag object ${REMOTE_TAG_OBJECT} does not match expected ${EXPECTED_TAG_OBJECT}"
fi
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

if [[ -n "${RC_NUMBER:-}" || "${TAG}" == "v${VERSION}" ]]; then
  RC_TAG_GLOB="refs/tags/v${VERSION}-rc.*"
  if ALL_RC_REFS="$(git ls-remote --exit-code --tags --refs "${REMOTE}" "${RC_TAG_GLOB}")"; then
    :
  else
    RC_QUERY_STATUS=$?
    if [[ "${RC_QUERY_STATUS}" -eq 2 && "${TAG}" == "v${VERSION}" ]]; then
      fail "final release tag ${TAG} requires at least one retained RC tag"
    fi
    fail "could not query the remote RC tag sequence from ${REMOTE}"
  fi

  RC_TAG_COUNT=0
  CURRENT_RC_COUNT=0
  MAX_OTHER_RC=0
  LOWEST_RC_NUMBER=999999999
  HIGHEST_RC_NUMBER=0
  HIGHEST_RC_OBJECT=""
  while read -r object_id ref_name; do
    [[ "${#object_id}" -eq 40 && "${object_id}" =~ ^[0-9a-f]+$ ]] ||
      fail "remote RC tag has an invalid Git object ID: ${object_id}"
    case "${ref_name}" in
      "refs/tags/v${VERSION}-rc."*)
        remote_rc_number="${ref_name#"refs/tags/v${VERSION}-rc."}"
        ;;
      *)
        fail "remote advertised an unexpected RC ref: ${ref_name}"
        ;;
    esac
    [[ "${remote_rc_number}" =~ ^[1-9][0-9]*$ && "${#remote_rc_number}" -le 9 ]] ||
      fail "remote advertised a malformed RC tag: ${ref_name}"
    if [[ "${VERSION}" == "1.0.0" && "${remote_rc_number}" -eq 1 ]]; then
      fail "v1.0.0-rc.1 is permanently burned and must remain absent"
    fi
    RC_TAG_COUNT=$((RC_TAG_COUNT + 1))
    if [[ "${remote_rc_number}" -gt "${HIGHEST_RC_NUMBER}" ]]; then
      HIGHEST_RC_NUMBER="${remote_rc_number}"
      HIGHEST_RC_OBJECT="${object_id}"
    fi
    if [[ "${remote_rc_number}" -lt "${LOWEST_RC_NUMBER}" ]]; then
      LOWEST_RC_NUMBER="${remote_rc_number}"
    fi
    if [[ -n "${RC_NUMBER:-}" && "${remote_rc_number}" -eq "${RC_NUMBER}" ]]; then
      CURRENT_RC_COUNT=$((CURRENT_RC_COUNT + 1))
      [[ "${object_id}" == "${REMOTE_TAG_OBJECT}" ]] ||
        fail "remote tag ${TAG} changed while validating the RC sequence"
    elif [[ -n "${RC_NUMBER:-}" && "${remote_rc_number}" -gt "${MAX_OTHER_RC}" ]]; then
      MAX_OTHER_RC="${remote_rc_number}"
    fi
  done <<< "${ALL_RC_REFS}"

  if [[ -n "${RC_NUMBER:-}" ]]; then
    [[ "${CURRENT_RC_COUNT}" -eq 1 ]] ||
      fail "remote must advertise ${TAG_REF} exactly once in the RC sequence"
    if [[ "${VERSION}" == "1.0.0" && "${RC_NUMBER}" -eq 2 && "${MAX_OTHER_RC}" -eq 0 ]]; then
      EXPECTED_RC_NUMBER=2
    else
      EXPECTED_RC_NUMBER=$((MAX_OTHER_RC + 1))
    fi
    [[ "${RC_NUMBER}" -eq "${EXPECTED_RC_NUMBER}" ]] ||
      fail "release tag ${TAG} is not the next RC after rc.${MAX_OTHER_RC}; expected rc.${EXPECTED_RC_NUMBER}"
  fi

  if [[ "${VERSION}" == "1.0.0" ]]; then
    [[ "${LOWEST_RC_NUMBER}" -eq 2 ]] ||
      fail "retained v1.0.0 RC sequence must start at rc.2"
  else
    [[ "${LOWEST_RC_NUMBER}" -eq 1 ]] ||
      fail "remote RC sequence starts unexpectedly at rc.${LOWEST_RC_NUMBER}"
  fi
  EXPECTED_RETAINED_RC_COUNT=$((HIGHEST_RC_NUMBER - LOWEST_RC_NUMBER + 1))
  [[ "${RC_TAG_COUNT}" -eq "${EXPECTED_RETAINED_RC_COUNT}" ]] ||
    fail "remote RC sequence has an internal gap or deleted retained tag: found ${RC_TAG_COUNT} tags from rc.${LOWEST_RC_NUMBER} through rc.${HIGHEST_RC_NUMBER}"

  if [[ "${TAG}" == "v${VERSION}" ]]; then
    HIGHEST_RC_REF="refs/tags/v${VERSION}-rc.${HIGHEST_RC_NUMBER}"
    HIGHEST_RC_PEELED_REF="${HIGHEST_RC_REF}^{}"
    if HIGHEST_RC_REFS="$(git ls-remote --exit-code --tags "${REMOTE}" \
      "${HIGHEST_RC_REF}" "${HIGHEST_RC_PEELED_REF}")"; then
      :
    else
      fail "could not query highest retained RC tag: ${HIGHEST_RC_REF}"
    fi

    HIGHEST_DIRECT_COUNT=0
    HIGHEST_PEELED_COUNT=0
    HIGHEST_PEELED_COMMIT=""
    while read -r object_id ref_name; do
      case "${ref_name}" in
        "${HIGHEST_RC_REF}")
          HIGHEST_DIRECT_COUNT=$((HIGHEST_DIRECT_COUNT + 1))
          [[ "${object_id}" == "${HIGHEST_RC_OBJECT}" ]] ||
            fail "highest retained RC changed while validating final release"
          ;;
        "${HIGHEST_RC_PEELED_REF}")
          HIGHEST_PEELED_COUNT=$((HIGHEST_PEELED_COUNT + 1))
          HIGHEST_PEELED_COMMIT="${object_id}"
          ;;
        *)
          fail "remote advertised an unexpected highest-RC ref: ${ref_name}"
          ;;
      esac
    done <<< "${HIGHEST_RC_REFS}"

    [[ "${HIGHEST_DIRECT_COUNT}" -eq 1 ]] ||
      fail "remote must advertise ${HIGHEST_RC_REF} exactly once"
    [[ "${HIGHEST_PEELED_COUNT}" -eq 1 ]] ||
      fail "highest retained RC ${HIGHEST_RC_REF} must be annotated"
    [[ "${#HIGHEST_PEELED_COMMIT}" -eq 40 && "${HIGHEST_PEELED_COMMIT}" =~ ^[0-9a-f]+$ ]] ||
      fail "highest retained RC has an invalid peeled commit: ${HIGHEST_PEELED_COMMIT}"
    [[ "${HIGHEST_RC_OBJECT}" != "${HIGHEST_PEELED_COMMIT}" ]] ||
      fail "highest retained RC ${HIGHEST_RC_REF} must resolve through a distinct tag object"
    [[ "${HIGHEST_PEELED_COMMIT}" == "${EXPECTED_COMMIT}" ]] ||
      fail "final release ${TAG} must use the same commit as ${HIGHEST_RC_REF}: ${HIGHEST_PEELED_COMMIT}"
  fi
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'tag_object=%s\n' "${REMOTE_TAG_OBJECT}" >> "${GITHUB_OUTPUT}"
fi

echo "release tag contract passed: ${TAG} -> ${REMOTE_PEELED_COMMIT} on ${REMOTE}/main"
