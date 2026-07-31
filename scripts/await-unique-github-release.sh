#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "GitHub release lookup failed: $*" >&2
  exit 1
}

REPOSITORY="${1:-}"
TAG="${2:-}"
ATTEMPTS="${SIGILLUM_RELEASE_LOOKUP_ATTEMPTS:-10}"
DELAY_SECONDS="${SIGILLUM_RELEASE_LOOKUP_DELAY_SECONDS:-2}"

[[ "${REPOSITORY}" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] ||
  fail "repository must be owner/name"
[[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[1-9][0-9]*)?$ ]] ||
  fail "release tag syntax is invalid: ${TAG}"
[[ "${ATTEMPTS}" =~ ^[1-9][0-9]*$ ]] ||
  fail "SIGILLUM_RELEASE_LOOKUP_ATTEMPTS must be a positive integer"
[[ "${DELAY_SECONDS}" =~ ^[0-9]+$ ]] ||
  fail "SIGILLUM_RELEASE_LOOKUP_DELAY_SECONDS must be a non-negative integer"

command -v gh >/dev/null 2>&1 || fail "gh is required"
command -v jq >/dev/null 2>&1 || fail "jq is required"

for ((attempt = 1; attempt <= ATTEMPTS; attempt++)); do
  release_pages="$(
    gh api --paginate "repos/${REPOSITORY}/releases?per_page=100"
  )" || fail "could not list releases for ${REPOSITORY}"

  matching_releases="$(
    jq -sc --arg tag "${TAG}" '
      if length == 0
      then error("release list returned no pages")
      elif any(.[]; type != "array")
      then error("release pages must be JSON arrays")
      elif any(.[][]; type != "object" or (.tag_name | type) != "string")
      then error("release items must be objects with string tag_name fields")
      else add
      end
      | [.[] | select(.tag_name == $tag)]
    ' <<< "${release_pages}"
  )" || fail "release list response was not valid paginated JSON"

  match_count="$(jq -r 'length' <<< "${matching_releases}")"
  case "${match_count}" in
    1)
      jq -c '.[0]' <<< "${matching_releases}"
      exit 0
      ;;
    0)
      if [[ "${attempt}" -lt "${ATTEMPTS}" ]]; then
        printf \
          'waiting for GitHub release %s to become visible (%s/%s)\n' \
          "${TAG}" "${attempt}" "${ATTEMPTS}" >&2
        sleep "${DELAY_SECONDS}"
        continue
      fi
      fail \
        "release ${TAG} was not visible after ${ATTEMPTS} attempts"
      ;;
    *)
      fail \
        "expected exactly one release for ${TAG}, found ${match_count}"
      ;;
  esac
done

fail "release lookup exhausted unexpectedly"
