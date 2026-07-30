#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "release state contract failed: $*" >&2
  exit 1
}

ROLE="${1:-}"
TAG="${2:-}"
RELEASE_JSON="${3:--}"

case "${ROLE}" in
  rc-draft | final-draft | final-published)
    ;;
  *)
    fail "role must be rc-draft, final-draft, or final-published"
    ;;
esac

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" ||
  fail "not inside a git repository"
VERSION="$(awk -F '"' '/^version = "/ { print $2; exit }' "${ROOT}/Cargo.toml")"
[[ -n "${VERSION}" ]] ||
  fail "workspace version is missing from Cargo.toml"

if [[ "${ROLE}" == "rc-draft" ]]; then
  [[ "${TAG}" =~ ^v${VERSION//./\.}-rc\.[1-9][0-9]*$ ]] ||
    fail "RC draft tag does not match workspace version ${VERSION}: ${TAG}"
else
  [[ "${TAG}" == "v${VERSION}" ]] ||
    fail "final release tag does not match workspace version ${VERSION}: ${TAG}"
fi

if [[ "${RELEASE_JSON}" == "-" ]]; then
  RELEASE_JSON="/dev/stdin"
else
  [[ -f "${RELEASE_JSON}" ]] ||
    fail "release JSON does not exist: ${RELEASE_JSON}"
fi

case "${ROLE}" in
  rc-draft)
    jq -e --arg tag "${TAG}" '
      type == "object" and
      .tag_name == $tag and
      .draft == true and
      .prerelease == true and
      .published_at == null
    ' "${RELEASE_JSON}" >/dev/null ||
      fail "RC release must be an unpublished prerelease draft"
    ;;
  final-draft)
    jq -e --arg tag "${TAG}" '
      type == "object" and
      .tag_name == $tag and
      .draft == true and
      .prerelease == false and
      .published_at == null
    ' "${RELEASE_JSON}" >/dev/null ||
      fail "final release draft must be unpublished and not a prerelease"
    ;;
  final-published)
    jq -e --arg tag "${TAG}" '
      type == "object" and
      .tag_name == $tag and
      .draft == false and
      .prerelease == false and
      (.published_at |
        type == "string" and
        test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    ' "${RELEASE_JSON}" >/dev/null ||
      fail "final release must be published and not a prerelease"
    ;;
esac

printf 'release state contract passed: %s %s\n' "${ROLE}" "${TAG}"
