#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="${ROOT}/scripts/check-macos-signing-env.sh"
tmp_parent="${TMPDIR:-/tmp}"
if [[ ! -d "${tmp_parent}" || ! -w "${tmp_parent}" ]]; then
  tmp_parent="/tmp"
fi
TMP_ROOT="$(mktemp -d "${tmp_parent%/}/sigillum-signing-env.XXXXXX")"
trap 'rm -rf "${TMP_ROOT}"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  echo "macOS signing environment regression failed: $*" >&2
  exit 1
}

run_success() {
  local label="$1"
  local expected="$2"
  shift 2
  local output=""
  output="$(env -i PATH="${PATH}" HOME="${HOME:-/tmp}" "$@" "${CHECK}")" || \
    fail "${label}: expected success"
  if [[ "${output}" != "${expected}" ]]; then
    fail "${label}: expected mode ${expected}, got ${output}"
  fi
}

run_failure() {
  local label="$1"
  shift
  if env -i PATH="${PATH}" HOME="${HOME:-/tmp}" "$@" "${CHECK}" >/dev/null 2>&1; then
    fail "${label}: expected failure"
  fi
}

api_key_path="${TMP_ROOT}/API key with spaces.p8"
printf '%s\n' 'test-key-material' >"${api_key_path}"
empty_api_key_path="${TMP_ROOT}/empty.p8"
: >"${empty_api_key_path}"
symlink_api_key_path="${TMP_ROOT}/symlink.p8"
ln -s "${api_key_path}" "${symlink_api_key_path}"

run_success "clean default" adhoc
run_success "whitespace is absent" adhoc \
  "APPLE_CERTIFICATE=   " "APPLE_CERTIFICATE_PASSWORD= " "APPLE_SIGNING_IDENTITY=  "
run_success "explicit ad-hoc" adhoc "APPLE_SIGNING_IDENTITY=-"
run_success "complete Developer ID without notarization" developer-id \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example (TEAM123456)"
run_success "complete Apple ID notarization" developer-id \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example (TEAM123456)" \
  "APPLE_ID=developer@example.com" "APPLE_PASSWORD=app-password" "APPLE_TEAM_ID=TEAM123456"
run_success "complete API-key notarization" developer-id \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example (TEAM123456)" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer" "APPLE_API_KEY_PATH=${api_key_path}"

run_failure "identity without certificate" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example (TEAM123456)"
run_failure "certificate without password or identity" "APPLE_CERTIFICATE=certificate"
run_failure "certificate and password without identity" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password"
run_failure "certificate and identity without password" \
  "APPLE_CERTIFICATE=certificate" "APPLE_SIGNING_IDENTITY=Developer ID Application: Example"
run_failure "ad-hoc with certificate fields" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" "APPLE_SIGNING_IDENTITY=-"
run_failure "partial Apple ID notarization" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_ID=developer@example.com" "APPLE_PASSWORD=app-password"
run_failure "partial API-key notarization" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer"
run_failure "both notarization families" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_ID=developer@example.com" "APPLE_PASSWORD=app-password" "APPLE_TEAM_ID=TEAM123456" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer" "APPLE_API_KEY_PATH=${api_key_path}"
run_failure "notarization with ad-hoc signing" \
  "APPLE_SIGNING_IDENTITY=-" \
  "APPLE_ID=developer@example.com" "APPLE_PASSWORD=app-password" "APPLE_TEAM_ID=TEAM123456"
run_failure "missing API key file" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer" "APPLE_API_KEY_PATH=${TMP_ROOT}/missing.p8"
run_failure "empty API key file" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer" "APPLE_API_KEY_PATH=${empty_api_key_path}"
run_failure "symlink API key file" \
  "APPLE_CERTIFICATE=certificate" "APPLE_CERTIFICATE_PASSWORD=password" \
  "APPLE_SIGNING_IDENTITY=Developer ID Application: Example" \
  "APPLE_API_KEY=KEY123" "APPLE_API_ISSUER=issuer" "APPLE_API_KEY_PATH=${symlink_api_key_path}"

echo "macOS signing environment regressions passed"
