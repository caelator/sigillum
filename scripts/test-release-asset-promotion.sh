#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROMOTER="${ROOT}/scripts/promote-release-assets.sh"
TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-promotion-test.XXXXXX")"
trap 'rm -rf "${TEMP_ROOT}"' EXIT

RC_TAG="v1.0.0-rc.6"
FINAL_TAG="v1.0.0"
VALID_RC="${TEMP_ROOT}/valid-rc"
VALID_FINAL="${TEMP_ROOT}/valid-final"

fail() {
  printf 'release asset promotion test failed: %s\n' "$*" >&2
  exit 1
}

write_sums() {
  local directory="$1"
  local sums_tmp="${directory}/SHA256SUMS.tmp"
  local asset
  : > "${sums_tmp}"
  for asset in \
    "${directory}"/Sigillum-*.app.zip \
    "${directory}"/Sigillum-*.dmg \
    "${directory}"/THIRD-PARTY-NOTICES.txt \
    "${directory}"/sigillum-cli-*.tar.gz; do
    shasum -a 256 "${asset}" |
      awk -v name="${asset##*/}" '{ print $1 "  " name }' >> "${sums_tmp}"
  done
  LC_ALL=C sort "${sums_tmp}" > "${directory}/SHA256SUMS"
  rm "${sums_tmp}"
}

make_valid_rc() {
  local directory="$1"
  mkdir -p "${directory}"
  printf 'app-zip\000exact-rc-payload\n' \
    > "${directory}/Sigillum-${RC_TAG}-macos-aarch64.app.zip"
  printf 'dmg\000exact-rc-payload\n' \
    > "${directory}/Sigillum-${RC_TAG}-macos-aarch64.dmg"
  printf 'notices\nexact-rc-payload\n' \
    > "${directory}/THIRD-PARTY-NOTICES.txt"
  printf 'linux-cli\000exact-rc-payload\n' \
    > "${directory}/sigillum-cli-${RC_TAG}-linux-x86_64.tar.gz"
  printf 'macos-cli\000exact-rc-payload\n' \
    > "${directory}/sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz"
  write_sums "${directory}"
}

clone_case() {
  local name="$1"
  local directory="${TEMP_ROOT}/${name}"
  cp -R "${VALID_RC}" "${directory}"
  printf '%s\n' "${directory}"
}

expect_failure() {
  local name="$1"
  local expected="$2"
  shift 2
  local log="${TEMP_ROOT}/${name}.log"
  if "$@" > "${log}" 2>&1; then
    fail "${name} unexpectedly passed"
  fi
  grep -F "${expected}" "${log}" >/dev/null ||
    fail "${name} did not report expected failure: ${expected}"
}

make_valid_rc "${VALID_RC}"
bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${VALID_RC}" "${VALID_FINAL}"
bash "${PROMOTER}" verify \
  "${RC_TAG}" "${FINAL_TAG}" "${VALID_RC}" "${VALID_FINAL}"

EXPECTED_NAMES="${TEMP_ROOT}/expected-names"
ACTUAL_NAMES="${TEMP_ROOT}/actual-names"
printf '%s\n' \
  "SHA256SUMS" \
  "Sigillum-${FINAL_TAG}-macos-aarch64.app.zip" \
  "Sigillum-${FINAL_TAG}-macos-aarch64.dmg" \
  "THIRD-PARTY-NOTICES.txt" \
  "sigillum-cli-${FINAL_TAG}-linux-x86_64.tar.gz" \
  "sigillum-cli-${FINAL_TAG}-macos-aarch64.tar.gz" |
  LC_ALL=C sort > "${EXPECTED_NAMES}"
find "${VALID_FINAL}" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
  LC_ALL=C sort > "${ACTUAL_NAMES}"
cmp -s "${EXPECTED_NAMES}" "${ACTUAL_NAMES}" ||
  fail "promoted asset names do not match the final contract"
(
  cd "${VALID_FINAL}"
  shasum -a 256 --check SHA256SUMS >/dev/null
)

for mapping in \
  "Sigillum-${RC_TAG}-macos-aarch64.app.zip|Sigillum-${FINAL_TAG}-macos-aarch64.app.zip" \
  "Sigillum-${RC_TAG}-macos-aarch64.dmg|Sigillum-${FINAL_TAG}-macos-aarch64.dmg" \
  "THIRD-PARTY-NOTICES.txt|THIRD-PARTY-NOTICES.txt" \
  "sigillum-cli-${RC_TAG}-linux-x86_64.tar.gz|sigillum-cli-${FINAL_TAG}-linux-x86_64.tar.gz" \
  "sigillum-cli-${RC_TAG}-macos-aarch64.tar.gz|sigillum-cli-${FINAL_TAG}-macos-aarch64.tar.gz"; do
  RC_NAME="${mapping%%|*}"
  FINAL_NAME="${mapping#*|}"
  cmp -s "${VALID_RC}/${RC_NAME}" "${VALID_FINAL}/${FINAL_NAME}" ||
    fail "payload bytes changed for ${FINAL_NAME}"
done

expect_failure "existing-output" "final assets path already exists" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${VALID_RC}" "${VALID_FINAL}"
expect_failure "malformed-rc-tag" "RC tag syntax is invalid" \
  bash "${PROMOTER}" verify \
  "v1.0.0-rc.06" "${FINAL_TAG}" "${VALID_RC}" "${VALID_FINAL}"
expect_failure "wrong-final-tag" "does not match RC base version" \
  bash "${PROMOTER}" verify \
  "${RC_TAG}" "v1.0.1" "${VALID_RC}" "${VALID_FINAL}"

TAMPERED_RC="$(clone_case tampered-rc)"
printf 'tamper\n' >> "${TAMPERED_RC}/Sigillum-${RC_TAG}-macos-aarch64.dmg"
expect_failure "tampered-rc" "payload digest does not match SHA256SUMS" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${TAMPERED_RC}" "${TEMP_ROOT}/tampered-output"

EXTRA_RC="$(clone_case extra-rc)"
printf 'unexpected\n' > "${EXTRA_RC}/unexpected.txt"
expect_failure "extra-rc" "must contain exactly five payloads and SHA256SUMS" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${EXTRA_RC}" "${TEMP_ROOT}/extra-output"

LINKED_RC="$(clone_case linked-rc)"
rm "${LINKED_RC}/THIRD-PARTY-NOTICES.txt"
ln -s "Sigillum-${RC_TAG}-macos-aarch64.dmg" \
  "${LINKED_RC}/THIRD-PARTY-NOTICES.txt"
expect_failure "linked-rc" "regular non-symlink file" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${LINKED_RC}" "${TEMP_ROOT}/linked-output"

MALFORMED_SUMS="$(clone_case malformed-sums)"
printf 'not-a-checksum\n' > "${MALFORMED_SUMS}/SHA256SUMS"
expect_failure "malformed-sums" "SHA256SUMS contains a malformed line" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${MALFORMED_SUMS}" "${TEMP_ROOT}/malformed-output"

DUPLICATE_SUMS="$(clone_case duplicate-sums)"
DUPLICATE_LINE="$(sed -n '1p' "${DUPLICATE_SUMS}/SHA256SUMS")"
printf '%s\n' "${DUPLICATE_LINE}" >> "${DUPLICATE_SUMS}/SHA256SUMS"
expect_failure "duplicate-sums" "SHA256SUMS contains a duplicate asset" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${DUPLICATE_SUMS}" "${TEMP_ROOT}/duplicate-output"

UNSAFE_SUMS="$(clone_case unsafe-sums)"
FIRST_DIGEST="$(awk 'NR == 1 { print $1 }' "${UNSAFE_SUMS}/SHA256SUMS")"
printf '%s  ../outside\n' "${FIRST_DIGEST}" > "${UNSAFE_SUMS}/SHA256SUMS"
expect_failure "unsafe-sums" "SHA256SUMS contains a malformed line" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${UNSAFE_SUMS}" "${TEMP_ROOT}/unsafe-output"

NESTED_OUTPUT="$(clone_case nested-output)"
expect_failure "nested-output" "must not be nested inside" \
  bash "${PROMOTER}" promote \
  "${RC_TAG}" "${FINAL_TAG}" "${NESTED_OUTPUT}" "${NESTED_OUTPUT}/final"

TAMPERED_FINAL="${TEMP_ROOT}/tampered-final"
cp -R "${VALID_FINAL}" "${TAMPERED_FINAL}"
printf 'replacement final payload\n' \
  > "${TAMPERED_FINAL}/Sigillum-${FINAL_TAG}-macos-aarch64.dmg"
write_sums "${TAMPERED_FINAL}"
expect_failure "tampered-final" "normalized RC and final payload digest manifests differ" \
  bash "${PROMOTER}" verify \
  "${RC_TAG}" "${FINAL_TAG}" "${VALID_RC}" "${TAMPERED_FINAL}"

MISSING_FINAL="${TEMP_ROOT}/missing-final"
cp -R "${VALID_FINAL}" "${MISSING_FINAL}"
rm "${MISSING_FINAL}/sigillum-cli-${FINAL_TAG}-linux-x86_64.tar.gz"
expect_failure "missing-final" "must contain exactly five payloads and SHA256SUMS" \
  bash "${PROMOTER}" verify \
  "${RC_TAG}" "${FINAL_TAG}" "${VALID_RC}" "${MISSING_FINAL}"

echo "release asset promotion tests passed"
