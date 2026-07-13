#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="${ROOT}/scripts/check-macos-bundle-signature.sh"

fail() {
  echo "macOS bundle signature regression failed: $*" >&2
  exit 1
}

usage() {
  echo "usage: $0 --mode <adhoc|developer-id> <Sigillum.app> <Sigillum.dmg>" >&2
  exit 2
}

if [[ "${1:-}" != "--mode" || "$#" != "4" ]]; then
  usage
fi

mode="$2"
source_app="$3"
source_dmg="$4"

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "bundle-signature regressions are supported only on macOS"
fi

tmp_parent="${TMPDIR:-/tmp}"
if [[ ! -d "${tmp_parent}" || ! -w "${tmp_parent}" ]]; then
  tmp_parent="/tmp"
fi
TMP_ROOT="$(mktemp -d "${tmp_parent%/}/sigillum signature tests.XXXXXX")"
trap 'rm -rf "${TMP_ROOT}"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

copy_app() {
  local destination="$1"
  ditto "${source_app}" "${destination}"
}

expect_failure() {
  local label="$1"
  local expected_text="$2"
  shift 2
  local output=""
  if output="$("$@" 2>&1)"; then
    fail "${label}: expected failure"
  fi
  if ! grep -Fq "${expected_text}" <<<"${output}"; then
    echo "${output}" >&2
    fail "${label}: failure did not contain '${expected_text}'"
  fi
}

create_dmg() {
  local source_dir="$1"
  local output_path="$2"
  hdiutil create -quiet -ov -fs HFS+ -volname "Sigillum Test" \
    -srcfolder "${source_dir}" "${output_path}"
}

"${CHECK}" --mode "${mode}" "${source_app}" "${source_dmg}" >/dev/null

spaces_dir="${TMP_ROOT}/valid pair with spaces"
mkdir -p "${spaces_dir}"
copy_app "${spaces_dir}/Sigillum.app"
cp "${source_dmg}" "${spaces_dir}/Sigillum valid image.dmg"
"${CHECK}" --mode "${mode}" "${spaces_dir}/Sigillum.app" \
  "${spaces_dir}/Sigillum valid image.dmg" >/dev/null

rc3_app="${TMP_ROOT}/rc3 shape/Sigillum.app"
mkdir -p "${rc3_app}/Contents/MacOS" "${rc3_app}/Contents/Resources"
cp "${source_app}/Contents/Info.plist" "${rc3_app}/Contents/Info.plist"
printf '%s\n' 'int main(void) { return 0; }' >"${TMP_ROOT}/rc3-linker-fixture.c"
xcrun clang -Wl,-adhoc_codesign -o \
  "${rc3_app}/Contents/MacOS/sigillum-desktop" \
  "${TMP_ROOT}/rc3-linker-fixture.c"
printf '%s\n' 'unsealed fixture resource' >"${rc3_app}/Contents/Resources/fixture.txt"
rc3_metadata="$(codesign -dv --verbose=4 "${rc3_app}" 2>&1)"
if ! grep -Fqx 'Signature=adhoc' <<<"${rc3_metadata}" || \
   ! grep -Fqx 'Info.plist=not bound' <<<"${rc3_metadata}" || \
   ! grep -Fqx 'Sealed Resources=none' <<<"${rc3_metadata}" || \
   ! grep -Eq '^CodeDirectory .*linker-signed' <<<"${rc3_metadata}"; then
  echo "${rc3_metadata}" >&2
  fail "RC3-shaped fixture does not reproduce the weak-gate metadata"
fi
expect_failure "RC3-shaped linker/binary-only signature" "CodeResources" \
  "${CHECK}" --mode adhoc "${rc3_app}" "${source_dmg}"

missing_seal_app="${TMP_ROOT}/missing seal/Sigillum.app"
mkdir -p "$(dirname "${missing_seal_app}")"
copy_app "${missing_seal_app}"
rm -f "${missing_seal_app}/Contents/_CodeSignature/CodeResources"
expect_failure "missing CodeResources" "CodeResources" \
  "${CHECK}" --mode "${mode}" "${missing_seal_app}" "${source_dmg}"

tampered_app="${TMP_ROOT}/tampered resource/Sigillum.app"
mkdir -p "$(dirname "${tampered_app}")"
copy_app "${tampered_app}"
resource_path="$(find "${tampered_app}/Contents/Resources" -type f -print -quit)"
if [[ -z "${resource_path}" ]]; then
  fail "could not find a sealed resource for tamper regression"
fi
printf '\ntampered\n' >>"${resource_path}"
expect_failure "tampered sealed resource" "strict code-signature" \
  "${CHECK}" --mode "${mode}" "${tampered_app}" "${source_dmg}"

if [[ "${mode}" == "adhoc" ]]; then
  mismatch_app="${TMP_ROOT}/CDHash mismatch/Sigillum.app"
  mkdir -p "$(dirname "${mismatch_app}")"
  copy_app "${mismatch_app}"
  printf '%s\n' 'different sealed content' >"${mismatch_app}/Contents/Resources/cdhash-mismatch.txt"
  codesign --force -s - --options runtime "${mismatch_app}" >/dev/null 2>&1
  expect_failure "source/dmg CDHash mismatch" "CDHash values differ" \
    "${CHECK}" --mode adhoc "${mismatch_app}" "${source_dmg}"

  no_runtime_app="${TMP_ROOT}/missing hardened runtime/Sigillum.app"
  mkdir -p "$(dirname "${no_runtime_app}")"
  copy_app "${no_runtime_app}"
  codesign --force -s - "${no_runtime_app}" >/dev/null 2>&1
  expect_failure "missing hardened runtime" "hardened runtime" \
    "${CHECK}" --mode adhoc "${no_runtime_app}" "${source_dmg}"
fi

wrong_id_app="${TMP_ROOT}/wrong identifier/Sigillum.app"
mkdir -p "$(dirname "${wrong_id_app}")"
copy_app "${wrong_id_app}"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.sigillum.invalid' \
  "${wrong_id_app}/Contents/Info.plist"
codesign --force -s - --options runtime --identifier com.sigillum.invalid \
  "${wrong_id_app}" >/dev/null 2>&1
expect_failure "wrong bundle identifier" "identifier" \
  "${CHECK}" --mode "${mode}" "${wrong_id_app}" "${source_dmg}"

symlink_app="${TMP_ROOT}/symlink root/Sigillum.app"
mkdir -p "$(dirname "${symlink_app}")"
ln -s "${source_app}" "${symlink_app}"
expect_failure "app-root symlink escape" "non-symlink directory" \
  "${CHECK}" --mode "${mode}" "${symlink_app}" "${source_dmg}"

internal_link_app="${TMP_ROOT}/internal symlink/Sigillum.app"
mkdir -p "$(dirname "${internal_link_app}")"
copy_app "${internal_link_app}"
ln -s /etc/passwd "${internal_link_app}/Contents/Resources/escape"
expect_failure "internal symlink escape" "internal symlink" \
  "${CHECK}" --mode "${mode}" "${internal_link_app}" "${source_dmg}"

symlink_dmg_source="${TMP_ROOT}/symlink dmg source"
mkdir -p "${symlink_dmg_source}"
ln -s "${source_app}" "${symlink_dmg_source}/Sigillum.app"
symlink_dmg="${TMP_ROOT}/symlink app.dmg"
create_dmg "${symlink_dmg_source}" "${symlink_dmg}"
expect_failure "dmg app symlink escape" "non-symlink directory" \
  "${CHECK}" --mode "${mode}" "${source_app}" "${symlink_dmg}"

zero_source="${TMP_ROOT}/zero app source"
mkdir -p "${zero_source}"
printf '%s\n' 'no application here' >"${zero_source}/README.txt"
zero_dmg="${TMP_ROOT}/zero apps.dmg"
create_dmg "${zero_source}" "${zero_dmg}"
expect_failure "zero-app dmg" "exactly one top-level app" \
  "${CHECK}" --mode "${mode}" "${source_app}" "${zero_dmg}"

multiple_source="${TMP_ROOT}/multiple app source"
mkdir -p "${multiple_source}"
copy_app "${multiple_source}/Sigillum.app"
copy_app "${multiple_source}/Other.app"
multiple_dmg="${TMP_ROOT}/multiple apps.dmg"
create_dmg "${multiple_source}" "${multiple_dmg}"
expect_failure "multiple-app dmg" "exactly one top-level app" \
  "${CHECK}" --mode "${mode}" "${source_app}" "${multiple_dmg}"

wrong_name_source="${TMP_ROOT}/wrong name source"
mkdir -p "${wrong_name_source}"
copy_app "${wrong_name_source}/Other.app"
wrong_name_dmg="${TMP_ROOT}/wrong app name.dmg"
create_dmg "${wrong_name_source}" "${wrong_name_dmg}"
expect_failure "wrong app name in dmg" "must be named Sigillum.app" \
  "${CHECK}" --mode "${mode}" "${source_app}" "${wrong_name_dmg}"

echo "macOS bundle signature regressions passed"
