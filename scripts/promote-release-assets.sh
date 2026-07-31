#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'release asset promotion failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  printf 'usage: %s <promote|verify> <rc-tag> <final-tag> <rc-assets-dir> <final-assets-dir>\n' \
    "${0##*/}" >&2
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 ||
    fail "required command is missing: $1"
}

MODE="${1:-}"
RC_TAG="${2:-}"
FINAL_TAG="${3:-}"
RC_ASSETS_INPUT="${4:-}"
FINAL_ASSETS_INPUT="${5:-}"
[[ "$#" -eq 5 ]] || usage
case "${MODE}" in
  promote | verify) ;;
  *) usage ;;
esac

[[ "${RC_TAG}" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)-rc\.([1-9][0-9]{0,8})$ ]] ||
  fail "RC tag syntax is invalid: ${RC_TAG}"
EXPECTED_FINAL_TAG="v${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${BASH_REMATCH[3]}"
[[ "${FINAL_TAG}" == "${EXPECTED_FINAL_TAG}" ]] ||
  fail "final tag ${FINAL_TAG} does not match RC base version ${EXPECTED_FINAL_TAG}"

require_command awk
require_command cmp
require_command shasum
require_command sort

[[ -d "${RC_ASSETS_INPUT}" && ! -L "${RC_ASSETS_INPUT}" ]] ||
  fail "RC assets path must be a real directory"
RC_ASSETS="$(cd "${RC_ASSETS_INPUT}" && pwd -P)"

TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-promotion.XXXXXX")"
CREATED_OUTPUT=""
cleanup() {
  rm -rf "${TEMP_ROOT}"
  if [[ -n "${CREATED_OUTPUT}" ]]; then
    rm -rf "${CREATED_OUTPUT}"
  fi
}
trap cleanup EXIT

RC_PAYLOADS=(
  "Sigillum-${RC_TAG}-macos-aarch64.app.zip"
  "Sigillum-${RC_TAG}-macos-aarch64.dmg"
  "THIRD-PARTY-NOTICES.txt"
  "sigillum-cli-${RC_TAG}-linux-x86_64.tar.gz"
  "sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz"
)
FINAL_PAYLOADS=(
  "Sigillum-${FINAL_TAG}-macos-aarch64.app.zip"
  "Sigillum-${FINAL_TAG}-macos-aarch64.dmg"
  "THIRD-PARTY-NOTICES.txt"
  "sigillum-cli-${FINAL_TAG}-linux-x86_64.tar.gz"
  "sigillum-cli-${FINAL_TAG}-macos-aarch64.tar.gz"
)
NORMALIZED_PAYLOADS=(
  'Sigillum-@RELEASE_TAG@-macos-aarch64.app.zip'
  'Sigillum-@RELEASE_TAG@-macos-aarch64.dmg'
  'THIRD-PARTY-NOTICES.txt'
  'sigillum-cli-@RELEASE_TAG@-linux-x86_64.tar.gz'
  'sigillum-cli-@RELEASE_TAG@-macos-aarch64.tar.gz'
)

asset_index() {
  local role="$1"
  local asset_name="$2"
  local index
  for index in 0 1 2 3 4; do
    if [[ "${role}" == "RC" && "${RC_PAYLOADS[${index}]}" == "${asset_name}" ]] ||
      [[ "${role}" == "final" && "${FINAL_PAYLOADS[${index}]}" == "${asset_name}" ]]; then
      printf '%s\n' "${index}"
      return 0
    fi
  done
  return 1
}

validate_asset_directory() {
  local directory="$1"
  local role="$2"
  local canonical_manifest="$3"
  local expected_payloads=()
  local entries=()
  local entry
  local entry_name
  local index
  local line
  local digest
  local asset_name
  local actual_digest
  local line_count=0
  local seen_names
  local sorted_manifest

  if [[ "${role}" == "RC" ]]; then
    expected_payloads=("${RC_PAYLOADS[@]}")
  else
    expected_payloads=("${FINAL_PAYLOADS[@]}")
  fi

  [[ -d "${directory}" && ! -L "${directory}" ]] ||
    fail "${role} assets path must be a real directory"

  shopt -s nullglob
  entries=("${directory}"/* "${directory}"/.[!.]* "${directory}"/..?*)
  shopt -u nullglob
  [[ "${#entries[@]}" -eq 6 ]] ||
    fail "${role} assets directory must contain exactly five payloads and SHA256SUMS"

  for entry in "${entries[@]}"; do
    [[ -f "${entry}" && ! -L "${entry}" ]] ||
      fail "${role} asset must be a regular non-symlink file: ${entry##*/}"
    entry_name="${entry##*/}"
    if [[ "${entry_name}" == "SHA256SUMS" ]]; then
      continue
    fi
    asset_index "${role}" "${entry_name}" >/dev/null ||
      fail "${role} assets directory contains an unexpected file: ${entry_name}"
  done

  for entry_name in "${expected_payloads[@]}"; do
    [[ -s "${directory}/${entry_name}" && ! -L "${directory}/${entry_name}" ]] ||
      fail "${role} payload is missing, empty, or linked: ${entry_name}"
  done
  [[ -s "${directory}/SHA256SUMS" && ! -L "${directory}/SHA256SUMS" ]] ||
    fail "${role} SHA256SUMS is missing, empty, or linked"

  seen_names="$(mktemp "${TEMP_ROOT}/seen.XXXXXX")"
  : > "${canonical_manifest}"
  while IFS= read -r line || [[ -n "${line}" ]]; do
    line_count=$((line_count + 1))
    [[ "${line}" =~ ^([0-9a-f]{64})\ \ ([A-Za-z0-9._-]+)$ ]] ||
      fail "${role} SHA256SUMS contains a malformed line"
    digest="${BASH_REMATCH[1]}"
    asset_name="${BASH_REMATCH[2]}"
    index="$(asset_index "${role}" "${asset_name}")" ||
      fail "${role} SHA256SUMS names an unexpected asset: ${asset_name}"
    if grep -Fx "${asset_name}" "${seen_names}" >/dev/null; then
      fail "${role} SHA256SUMS contains a duplicate asset: ${asset_name}"
    fi
    printf '%s\n' "${asset_name}" >> "${seen_names}"
    actual_digest="$(shasum -a 256 "${directory}/${asset_name}" | awk '{print $1}')"
    [[ "${actual_digest}" == "${digest}" ]] ||
      fail "${role} payload digest does not match SHA256SUMS: ${asset_name}"
    printf '%s  %s\n' "${digest}" "${NORMALIZED_PAYLOADS[${index}]}" \
      >> "${canonical_manifest}"
  done < "${directory}/SHA256SUMS"

  [[ "${line_count}" -eq 5 ]] ||
    fail "${role} SHA256SUMS must contain exactly five payload entries"
  for entry_name in "${expected_payloads[@]}"; do
    grep -Fx "${entry_name}" "${seen_names}" >/dev/null ||
      fail "${role} SHA256SUMS is missing payload: ${entry_name}"
  done

  sorted_manifest="${canonical_manifest}.sorted"
  LC_ALL=C sort "${canonical_manifest}" > "${sorted_manifest}"
  mv "${sorted_manifest}" "${canonical_manifest}"
}

verify_promotion() {
  local final_assets="$1"
  local rc_manifest
  local final_manifest
  local index

  rc_manifest="$(mktemp "${TEMP_ROOT}/rc-manifest.XXXXXX")"
  final_manifest="$(mktemp "${TEMP_ROOT}/final-manifest.XXXXXX")"
  validate_asset_directory "${RC_ASSETS}" "RC" "${rc_manifest}"
  validate_asset_directory "${final_assets}" "final" "${final_manifest}"
  if ! cmp -s "${rc_manifest}" "${final_manifest}"; then
    fail "normalized RC and final payload digest manifests differ"
  fi
  for index in 0 1 2 3 4; do
    if ! cmp -s \
      "${RC_ASSETS}/${RC_PAYLOADS[${index}]}" \
      "${final_assets}/${FINAL_PAYLOADS[${index}]}"; then
      fail "promoted payload bytes differ: ${FINAL_PAYLOADS[${index}]}"
    fi
  done
}

if [[ "${MODE}" == "verify" ]]; then
  [[ -d "${FINAL_ASSETS_INPUT}" && ! -L "${FINAL_ASSETS_INPUT}" ]] ||
    fail "final assets path must be a real directory"
  FINAL_ASSETS="$(cd "${FINAL_ASSETS_INPUT}" && pwd -P)"
  [[ "${FINAL_ASSETS}" != "${RC_ASSETS}" ]] ||
    fail "RC and final assets directories must be distinct"
  verify_promotion "${FINAL_ASSETS}"
  printf 'release asset promotion verified: %s -> %s\n' "${RC_TAG}" "${FINAL_TAG}"
  exit 0
fi

FINAL_PARENT_INPUT="$(dirname "${FINAL_ASSETS_INPUT}")"
FINAL_BASENAME="$(basename "${FINAL_ASSETS_INPUT}")"
[[ "${FINAL_BASENAME}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] ||
  fail "final assets directory name is unsafe: ${FINAL_BASENAME}"
[[ -d "${FINAL_PARENT_INPUT}" && ! -L "${FINAL_PARENT_INPUT}" ]] ||
  fail "final assets parent must be a real directory"
FINAL_PARENT="$(cd "${FINAL_PARENT_INPUT}" && pwd -P)"
FINAL_ASSETS="${FINAL_PARENT}/${FINAL_BASENAME}"
[[ ! -e "${FINAL_ASSETS}" && ! -L "${FINAL_ASSETS}" ]] ||
  fail "final assets path already exists: ${FINAL_ASSETS}"
case "${FINAL_ASSETS}/" in
  "${RC_ASSETS}/"*)
    fail "final assets directory must not be nested inside the RC assets directory"
    ;;
esac

RC_MANIFEST="$(mktemp "${TEMP_ROOT}/rc-preflight.XXXXXX")"
validate_asset_directory "${RC_ASSETS}" "RC" "${RC_MANIFEST}"

mkdir "${FINAL_ASSETS}" ||
  fail "could not create final assets directory"
CREATED_OUTPUT="${FINAL_ASSETS}"

for index in 0 1 2 3 4; do
  cp \
    "${RC_ASSETS}/${RC_PAYLOADS[${index}]}" \
    "${FINAL_ASSETS}/${FINAL_PAYLOADS[${index}]}"
  cmp -s \
    "${RC_ASSETS}/${RC_PAYLOADS[${index}]}" \
    "${FINAL_ASSETS}/${FINAL_PAYLOADS[${index}]}" ||
    fail "copy changed payload bytes: ${FINAL_PAYLOADS[${index}]}"
done

UNSORTED_SUMS="$(mktemp "${TEMP_ROOT}/final-sums.XXXXXX")"
for index in 0 1 2 3 4; do
  FINAL_DIGEST="$(
    shasum -a 256 "${FINAL_ASSETS}/${FINAL_PAYLOADS[${index}]}" |
      awk '{print $1}'
  )"
  printf '%s  %s\n' "${FINAL_DIGEST}" "${FINAL_PAYLOADS[${index}]}" \
    >> "${UNSORTED_SUMS}"
done
LC_ALL=C sort "${UNSORTED_SUMS}" > "${FINAL_ASSETS}/SHA256SUMS"

verify_promotion "${FINAL_ASSETS}"
CREATED_OUTPUT=""
printf 'release assets promoted byte-for-byte: %s -> %s\n' "${RC_TAG}" "${FINAL_TAG}"
