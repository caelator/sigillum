#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="${ROOT}/scripts/notarize-macos-dmg.sh"
SIGNING_CHECK="${ROOT}/scripts/check-macos-signing-env.sh"
BUILD_WRAPPER="${ROOT}/scripts/build-macos-bundle.sh"

fail() {
  echo "macOS dmg notarization regression failed: $*" >&2
  exit 1
}

if ! command -v jq >/dev/null 2>&1; then
  fail "required command is missing: jq"
fi

tmp_parent="${TMPDIR:-/tmp}"
if [[ ! -d "${tmp_parent}" || ! -w "${tmp_parent}" ]]; then
  tmp_parent="/tmp"
fi
TMP_ROOT="$(mktemp -d "${tmp_parent%/}/sigillum-dmg-notary.XXXXXX")"
trap 'rm -rf "${TMP_ROOT}"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

FAKE_BIN="${TMP_ROOT}/fake bin"
mkdir -p "${FAKE_BIN}"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'printf "%s\n" Darwin' \
  >"${FAKE_BIN}/uname"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ "${1:-}" == "--verify" ]]; then' \
  '  exit "${SIGILLUM_CODESIGN_VERIFY_EXIT:-0}"' \
  'fi' \
  'if [[ "${1:-}" == "-dv" ]]; then' \
  '  if [[ "${SIGILLUM_CODESIGN_ADHOC:-0}" == "1" ]]; then' \
  '    printf "%s\n" "Signature=adhoc" "TeamIdentifier=not set" >&2' \
  '  else' \
  '    printf "%s\n" "Authority=Developer ID Application: Example (TEAM123456)" "TeamIdentifier=TEAM123456" >&2' \
  '  fi' \
  '  exit 0' \
  'fi' \
  'exit 64' \
  >"${FAKE_BIN}/codesign"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  ': "${SIGILLUM_XCRUN_LOG:?}"' \
  '{' \
  '  printf "%s\n" BEGIN' \
  '  printf "ARG=%s\n" "$@"' \
  '  printf "%s\n" END' \
  '} >>"${SIGILLUM_XCRUN_LOG}"' \
  'if [[ "${1:-}" == "notarytool" && "${2:-}" == "submit" ]]; then' \
  '  if [[ "${SIGILLUM_NOTARY_EXIT:-0}" != "0" ]]; then' \
  '    printf "%s\n" "simulated notarytool failure" >&2' \
  '    exit "${SIGILLUM_NOTARY_EXIT}"' \
  '  fi' \
  '  if [[ -n "${SIGILLUM_NOTARY_OUTPUT+x}" ]]; then' \
  '    printf "%s\n" "${SIGILLUM_NOTARY_OUTPUT}"' \
  '  else' \
  '    printf "%s\n" '\''{"id":"submission-1","status":"Accepted","message":"ok"}'\''' \
  '  fi' \
  '  exit 0' \
  'fi' \
  'if [[ "${1:-}" == "stapler" && "${2:-}" == "staple" ]]; then' \
  '  exit "${SIGILLUM_STAPLE_EXIT:-0}"' \
  'fi' \
  'if [[ "${1:-}" == "stapler" && "${2:-}" == "validate" ]]; then' \
  '  exit "${SIGILLUM_VALIDATE_EXIT:-0}"' \
  'fi' \
  'exit 64' \
  >"${FAKE_BIN}/xcrun"

chmod +x "${FAKE_BIN}/uname" "${FAKE_BIN}/codesign" "${FAKE_BIN}/xcrun"

JQ_DIR="$(cd "$(dirname "$(command -v jq)")" && pwd -P)"
TEST_PATH="${FAKE_BIN}:${JQ_DIR}:/usr/bin:/bin"
XCRUN_LOG="${TMP_ROOT}/xcrun args.log"
DMG_PATH="${TMP_ROOT}/Sigillum image with spaces.dmg"
printf '%s\n' 'fake dmg content' >"${DMG_PATH}"
API_KEY_PATH="${TMP_ROOT}/API key with spaces.p8"
printf '%s\n' 'fake api key content' >"${API_KEY_PATH}"
API_KEY_CANONICAL_DIR="$(cd "$(dirname "${API_KEY_PATH}")" && pwd -P)"
API_KEY_CANONICAL_PATH="${API_KEY_CANONICAL_DIR}/$(basename "${API_KEY_PATH}")"
SYMLINK_DMG="${TMP_ROOT}/symlink.dmg"
ln -s "${DMG_PATH}" "${SYMLINK_DMG}"

BASE_ENV=(
  "PATH=${TEST_PATH}"
  "HOME=${HOME:-/tmp}"
  "SIGILLUM_XCRUN_LOG=${XCRUN_LOG}"
  "APPLE_CERTIFICATE=certificate-secret-value"
  "APPLE_CERTIFICATE_PASSWORD=certificate-password-secret-value"
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example (TEAM123456)"
)

APPLE_ID_ENV=(
  "APPLE_ID=developer@example.com"
  "APPLE_PASSWORD=app-password-value"
  "APPLE_TEAM_ID=TEAM123456"
)

API_KEY_ENV=(
  "APPLE_API_KEY=KEY123"
  "APPLE_API_ISSUER=issuer-value"
  "APPLE_API_KEY_PATH=${API_KEY_PATH}"
)

run_success() {
  local label="$1"
  shift
  local output=""
  : >"${XCRUN_LOG}"
  if ! output="$(env -i "${BASE_ENV[@]}" "$@" "${CHECK}" "${DMG_PATH}" 2>&1)"; then
    echo "${output}" >&2
    fail "${label}: expected success"
  fi
  assert_no_fixture_secrets "${label}" "${output}"
}

run_failure() {
  local label="$1"
  local expected_text="$2"
  shift 2
  local output=""
  : >"${XCRUN_LOG}"
  if output="$(env -i "${BASE_ENV[@]}" "$@" "${CHECK}" "${DMG_PATH}" 2>&1)"; then
    fail "${label}: expected failure"
  fi
  if ! grep -Fq "${expected_text}" <<<"${output}"; then
    echo "${output}" >&2
    fail "${label}: failure did not contain '${expected_text}'"
  fi
}

assert_no_fixture_secrets() {
  local label="$1"
  local output="$2"
  local secret
  for secret in \
    certificate-secret-value certificate-password-secret-value \
    app-password-value "${API_KEY_PATH}" 'fake api key content' issuer-value
  do
    if grep -Fq "${secret}" <<<"${output}"; then
      fail "${label}: output leaked fixture credential material"
    fi
  done
}

assert_log_exact() {
  local label="$1"
  local expected_file="$2"
  if ! cmp -s "${expected_file}" "${XCRUN_LOG}"; then
    diff -u "${expected_file}" "${XCRUN_LOG}" >&2 || true
    fail "${label}: xcrun invocation grouping or argument order differed"
  fi
}

run_success "Apple ID credentials" "${APPLE_ID_ENV[@]}"
APPLE_EXPECTED_LOG="${TMP_ROOT}/apple-id expected.log"
printf '%s\n' \
  BEGIN \
  ARG=notarytool ARG=submit "ARG=${DMG_PATH}" ARG=--wait \
  ARG=--output-format ARG=json \
  ARG=--apple-id ARG=developer@example.com \
  ARG=--password ARG=app-password-value \
  ARG=--team-id ARG=TEAM123456 \
  END \
  BEGIN ARG=stapler ARG=staple ARG=-v "ARG=${DMG_PATH}" END \
  BEGIN ARG=stapler ARG=validate "ARG=${DMG_PATH}" END \
  >"${APPLE_EXPECTED_LOG}"
assert_log_exact "Apple ID credentials" "${APPLE_EXPECTED_LOG}"

run_success "API key credentials" "${API_KEY_ENV[@]}"
API_EXPECTED_LOG="${TMP_ROOT}/api-key expected.log"
printf '%s\n' \
  BEGIN \
  ARG=notarytool ARG=submit "ARG=${DMG_PATH}" ARG=--wait \
  ARG=--output-format ARG=json \
  ARG=--key-id ARG=KEY123 \
  ARG=--key "ARG=${API_KEY_CANONICAL_PATH}" \
  ARG=--issuer ARG=issuer-value \
  END \
  BEGIN ARG=stapler ARG=staple ARG=-v "ARG=${DMG_PATH}" END \
  BEGIN ARG=stapler ARG=validate "ARG=${DMG_PATH}" END \
  >"${API_EXPECTED_LOG}"
assert_log_exact "API key credentials" "${API_EXPECTED_LOG}"

run_failure "non-JSON response" "did not return an accepted submission" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_NOTARY_OUTPUT=not-json"
run_failure "rejected response" "did not return an accepted submission" \
  "${APPLE_ID_ENV[@]}" \
  'SIGILLUM_NOTARY_OUTPUT={"id":"submission-2","status":"Rejected","message":"no"}'
run_failure "missing submission id" "did not return an accepted submission" \
  "${APPLE_ID_ENV[@]}" \
  'SIGILLUM_NOTARY_OUTPUT={"id":"","status":"Accepted","message":"no id"}'
run_failure "notarytool exit" "notarytool submission failed" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_NOTARY_EXIT=1"
run_failure "staple exit" "could not staple" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_STAPLE_EXIT=1"
run_failure "validate exit" "did not validate" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_VALIDATE_EXIT=1"
run_failure "invalid dmg signature" "strict code-signature" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_CODESIGN_VERIFY_EXIT=1"
run_failure "ad-hoc dmg signature" "not Developer ID signed" \
  "${APPLE_ID_ENV[@]}" "SIGILLUM_CODESIGN_ADHOC=1"

symlink_output=""
if symlink_output="$(env -i "${BASE_ENV[@]}" "${APPLE_ID_ENV[@]}" \
  "${CHECK}" "${SYMLINK_DMG}" 2>&1)"; then
  fail "symlink dmg: expected failure"
fi
if ! grep -Fq 'non-symlink regular file' <<<"${symlink_output}"; then
  echo "${symlink_output}" >&2
  fail "symlink dmg: wrong failure"
fi

xtrace_output=""
if xtrace_output="$(env -i "${BASE_ENV[@]}" "${APPLE_ID_ENV[@]}" \
  bash -x "${CHECK}" "${DMG_PATH}" 2>&1)"; then
  fail "xtrace: expected failure"
fi
if ! grep -Fq 'xtrace is enabled' <<<"${xtrace_output}"; then
  echo "${xtrace_output}" >&2
  fail "xtrace: wrong failure"
fi
assert_no_fixture_secrets "notarization helper xtrace" "${xtrace_output}"

for guarded_script in "${SIGNING_CHECK}" "${BUILD_WRAPPER}"; do
  xtrace_output=""
  if xtrace_output="$(env -i "${BASE_ENV[@]}" "${APPLE_ID_ENV[@]}" \
    "${API_KEY_ENV[@]}" bash -x "${guarded_script}" --debug 2>&1)"; then
    fail "xtrace guard ${guarded_script}: expected failure"
  fi
  if ! grep -Fq 'xtrace is enabled' <<<"${xtrace_output}"; then
    echo "${xtrace_output}" >&2
    fail "xtrace guard ${guarded_script}: wrong failure"
  fi
  assert_no_fixture_secrets "xtrace guard ${guarded_script}" "${xtrace_output}"
done

echo "macOS dmg notarization regressions passed"
