#!/usr/bin/env bash
set -euo pipefail

EXPECTED_IDENTIFIER="com.sigillum.desktop"
EXPECTED_EXECUTABLE="sigillum-desktop"

fail() {
  echo "macOS bundle signature check failed: $*" >&2
  exit 1
}

usage() {
  echo "usage: $0 --mode <adhoc|developer-id> <Sigillum.app> <Sigillum.dmg>" >&2
  exit 2
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    fail "required command is missing: ${command_name}"
  fi
}

if [[ "${1:-}" != "--mode" || "$#" != "4" ]]; then
  usage
fi

mode="$2"
source_app="$3"
dmg_path="$4"

if [[ "${mode}" != "adhoc" && "${mode}" != "developer-id" ]]; then
  usage
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "bundle-signature verification is supported only on macOS"
fi

require_command codesign
require_command hdiutil
require_command mktemp
require_command /usr/libexec/PlistBuddy
if [[ "${mode}" == "developer-id" ]]; then
  require_command xcrun
fi

VERIFIED_CDHASH=""
VERIFIED_TEAM_ID=""

verify_app() {
  local app="$1"
  local label="$2"
  local contents="${app}/Contents"
  local info_plist="${contents}/Info.plist"
  local executable_dir="${contents}/MacOS"
  local executable="${executable_dir}/${EXPECTED_EXECUTABLE}"
  local signature_dir="${contents}/_CodeSignature"
  local code_resources="${signature_dir}/CodeResources"
  local metadata=""
  local verify_output=""
  local plist_identifier=""
  local plist_executable=""
  local symlink_path=""
  local cdhash_lines=""
  local staple_output=""

  if [[ ! -d "${app}" || -L "${app}" ]]; then
    fail "${label} app must be a non-symlink directory: ${app}"
  fi
  if [[ "$(basename "${app}")" != "Sigillum.app" ]]; then
    fail "${label} app must be named Sigillum.app"
  fi

  for critical_path in \
    "${contents}" "${info_plist}" "${executable_dir}" "${executable}" \
    "${signature_dir}" "${code_resources}"
  do
    if [[ -L "${critical_path}" ]]; then
      fail "${label} app contains a symlink at a signature-critical path: ${critical_path}"
    fi
  done

  symlink_path="$(find "${app}" -type l -print -quit)"
  if [[ -n "${symlink_path}" ]]; then
    fail "${label} app contains an unsupported internal symlink: ${symlink_path}"
  fi

  if [[ ! -f "${info_plist}" || ! -r "${info_plist}" ]]; then
    fail "${label} app is missing a readable Info.plist"
  fi
  if [[ ! -x "${executable}" || ! -f "${executable}" ]]; then
    fail "${label} app is missing the expected executable: ${executable}"
  fi
  if [[ ! -s "${code_resources}" || ! -f "${code_resources}" ]]; then
    fail "${label} app is missing a nonempty Contents/_CodeSignature/CodeResources seal"
  fi

  plist_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "${info_plist}" 2>/dev/null)" || \
    fail "${label} app Info.plist has no CFBundleIdentifier"
  plist_executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "${info_plist}" 2>/dev/null)" || \
    fail "${label} app Info.plist has no CFBundleExecutable"
  if [[ "${plist_identifier}" != "${EXPECTED_IDENTIFIER}" ]]; then
    fail "${label} app Info.plist identifier is ${plist_identifier}, expected ${EXPECTED_IDENTIFIER}"
  fi
  if [[ "${plist_executable}" != "${EXPECTED_EXECUTABLE}" ]]; then
    fail "${label} app Info.plist executable is ${plist_executable}, expected ${EXPECTED_EXECUTABLE}"
  fi

  if ! verify_output="$(codesign --verify --deep --strict --verbose=4 "${app}" 2>&1)"; then
    echo "${verify_output}" >&2
    fail "${label} app failed strict code-signature verification"
  fi

  if ! metadata="$(codesign -dv --verbose=4 "${app}" 2>&1)"; then
    echo "${metadata}" >&2
    fail "${label} app signature metadata could not be read"
  fi

  if ! grep -Fqx "Identifier=${EXPECTED_IDENTIFIER}" <<<"${metadata}"; then
    echo "${metadata}" >&2
    fail "${label} app code-directory identifier is not ${EXPECTED_IDENTIFIER}"
  fi
  if ! grep -Eq '^Info\.plist entries=[1-9][0-9]*$' <<<"${metadata}" || \
     grep -Fqx 'Info.plist=not bound' <<<"${metadata}"; then
    echo "${metadata}" >&2
    fail "${label} app Info.plist is not bound into the bundle signature"
  fi
  if ! grep -Eq '^Sealed Resources version=2([[:space:]]|$)' <<<"${metadata}" || \
     grep -Fqx 'Sealed Resources=none' <<<"${metadata}"; then
    echo "${metadata}" >&2
    fail "${label} app resources are not sealed"
  fi
  if grep -Eq '^CodeDirectory .*linker-signed' <<<"${metadata}"; then
    echo "${metadata}" >&2
    fail "${label} app has only a linker signature"
  fi
  if ! grep -Eq '^CodeDirectory .*\([^)]*runtime[^)]*\)' <<<"${metadata}" || \
     ! grep -Eq '^Runtime Version=.+' <<<"${metadata}"; then
    echo "${metadata}" >&2
    fail "${label} app signature does not enable the hardened runtime"
  fi

  if [[ "${mode}" == "adhoc" ]]; then
    if ! grep -Fqx 'Signature=adhoc' <<<"${metadata}" || \
       ! grep -Fqx 'TeamIdentifier=not set' <<<"${metadata}"; then
      echo "${metadata}" >&2
      fail "${label} app is not signed in the expected ad-hoc mode"
    fi
    VERIFIED_TEAM_ID="not set"
  else
    if grep -Fqx 'Signature=adhoc' <<<"${metadata}" || \
       ! grep -Eq '^Authority=Developer ID Application: .+' <<<"${metadata}" || \
       ! grep -Eq '^TeamIdentifier=[A-Z0-9]+$' <<<"${metadata}"; then
      echo "${metadata}" >&2
      fail "${label} app is not signed in the expected Developer ID mode"
    fi
    VERIFIED_TEAM_ID="$(sed -n 's/^TeamIdentifier=//p' <<<"${metadata}")"
    if ! staple_output="$(xcrun stapler validate "${app}" 2>&1)"; then
      echo "${staple_output}" >&2
      fail "${label} Developer ID app has no valid stapled notarization ticket"
    fi
  fi

  cdhash_lines="$(grep '^CDHash=' <<<"${metadata}")"
  if [[ "$(grep -c '^CDHash=' <<<"${metadata}")" != "1" ]]; then
    echo "${metadata}" >&2
    fail "${label} app must expose exactly one CDHash"
  fi
  VERIFIED_CDHASH="${cdhash_lines#CDHash=}"
}

if [[ ! -f "${dmg_path}" || -L "${dmg_path}" || ! -r "${dmg_path}" ]]; then
  fail "dmg must be a readable, non-symlink regular file: ${dmg_path}"
fi

verify_app "${source_app}" "source"
source_cdhash="${VERIFIED_CDHASH}"
source_team_id="${VERIFIED_TEAM_ID}"

if [[ "${mode}" == "developer-id" ]]; then
  dmg_verify_output=""
  dmg_metadata=""
  if ! dmg_verify_output="$(codesign --verify --strict --verbose=4 "${dmg_path}" 2>&1)"; then
    echo "${dmg_verify_output}" >&2
    fail "Developer ID dmg failed strict code-signature verification"
  fi
  if ! dmg_metadata="$(codesign -dv --verbose=4 "${dmg_path}" 2>&1)"; then
    echo "${dmg_metadata}" >&2
    fail "Developer ID dmg signature metadata could not be read"
  fi
  if grep -Fqx 'Signature=adhoc' <<<"${dmg_metadata}" || \
     ! grep -Eq '^Authority=Developer ID Application: .+' <<<"${dmg_metadata}" || \
     ! grep -Fqx "TeamIdentifier=${source_team_id}" <<<"${dmg_metadata}"; then
    echo "${dmg_metadata}" >&2
    fail "dmg is not Developer ID signed by the same team as the app"
  fi
fi

tmp_parent="${TMPDIR:-/tmp}"
if [[ ! -d "${tmp_parent}" || ! -w "${tmp_parent}" ]]; then
  tmp_parent="/tmp"
fi
mount_point="$(mktemp -d "${tmp_parent%/}/sigillum bundle verify.XXXXXX")"
mounted=0

cleanup() {
  if [[ "${mounted}" == "1" ]]; then
    hdiutil detach "${mount_point}" >/dev/null 2>&1 || \
      hdiutil detach -force "${mount_point}" >/dev/null 2>&1 || true
    mounted=0
  fi
  rmdir "${mount_point}" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

mounted=1
if ! hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "${mount_point}" "${dmg_path}" >/dev/null; then
  fail "could not mount dmg read-only: ${dmg_path}"
fi

mount_real="$(cd "${mount_point}" && pwd -P)"
app_paths=()
while IFS= read -r -d '' candidate; do
  app_paths+=("${candidate}")
done < <(find "${mount_point}" -mindepth 1 -maxdepth 1 -name '*.app' -print0)

if [[ "${#app_paths[@]}" != "1" ]]; then
  fail "mounted dmg must contain exactly one top-level app; found ${#app_paths[@]}"
fi
mounted_app="${app_paths[0]}"
if [[ "$(basename "${mounted_app}")" != "Sigillum.app" ]]; then
  fail "the single top-level app in the dmg must be named Sigillum.app"
fi
if [[ -L "${mounted_app}" || ! -d "${mounted_app}" ]]; then
  fail "mounted Sigillum.app must be a non-symlink directory"
fi
mounted_app_real="$(cd "${mounted_app}" && pwd -P)"
if [[ "${mounted_app_real}" != "${mount_real}/Sigillum.app" ]]; then
  fail "mounted Sigillum.app resolves outside the expected dmg layout"
fi

verify_app "${mounted_app}" "mounted dmg"
mounted_cdhash="${VERIFIED_CDHASH}"
if [[ "${source_cdhash}" != "${mounted_cdhash}" ]]; then
  fail "source and mounted-dmg app CDHash values differ"
fi

if ! hdiutil detach "${mount_point}" >/dev/null 2>&1 && \
   ! hdiutil detach -force "${mount_point}" >/dev/null 2>&1; then
  fail "could not detach verifier-created dmg mount"
fi
mounted=0
if ! rmdir "${mount_point}"; then
  fail "could not remove verifier-created mountpoint"
fi
trap - EXIT HUP INT TERM

echo "macOS bundle signature checks passed (${mode}, CDHash=${source_cdhash})"
