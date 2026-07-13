#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "macOS bundle build failed: $*" >&2
  exit 1
}

case "$-" in
  *x*) fail "refusing to handle signing credentials while shell xtrace is enabled" ;;
esac

trim_env() {
  local name="$1"
  local value="${!name-}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

normalize_text_env() {
  local name="$1"
  local value
  value="$(trim_env "${name}")"
  if [[ -n "${value}" ]]; then
    printf -v "${name}" '%s' "${value}"
    export "${name}"
  else
    unset "${name}"
  fi
}

find_bundle_root() {
  local profile="$1"
  local candidate
  for candidate in \
    "${ROOT}/target/${profile}/bundle" \
    "${ROOT}/crates/sigillum-desktop/target/${profile}/bundle"
  do
    if [[ -d "${candidate}/macos/Sigillum.app" && -d "${candidate}/dmg" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

remove_previous_dmgs() {
  local profile="$1"
  local dmg_dir
  local stale_dmg
  for dmg_dir in \
    "${ROOT}/target/${profile}/bundle/dmg" \
    "${ROOT}/crates/sigillum-desktop/target/${profile}/bundle/dmg"
  do
    if [[ ! -d "${dmg_dir}" ]]; then
      continue
    fi
    while IFS= read -r -d '' stale_dmg; do
      rm -f "${stale_dmg}"
    done < <(find "${dmg_dir}" -mindepth 1 -maxdepth 1 \
      -type f -name 'Sigillum_*.dmg' -print0)
  done
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "Tauri app/dmg bundling is supported only on macOS"
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  fail "required cargo subcommand is missing: cargo tauri. Install with: cargo install tauri-cli --version 2.11.4 --locked"
fi

signing_mode="$("${ROOT}/scripts/check-macos-signing-env.sh")"

for name in \
  APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID \
  APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
do
  normalize_text_env "${name}"
done

for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_PASSWORD; do
  if [[ -z "$(trim_env "${name}")" ]]; then
    unset "${name}"
  else
    export "${name}"
  fi
done

if [[ "${signing_mode}" == "adhoc" ]]; then
  export APPLE_SIGNING_IDENTITY="-"
else
  api_key_path="$(trim_env APPLE_API_KEY_PATH)"
  if [[ -n "${api_key_path}" ]]; then
    api_key_dir="$(cd "$(dirname "${api_key_path}")" && pwd -P)"
    export APPLE_API_KEY_PATH="${api_key_dir}/$(basename "${api_key_path}")"
  fi
fi

profile="release"
for arg in "$@"; do
  if [[ "${arg}" == "--" ]]; then
    break
  fi
  if [[ "${arg}" == "--debug" ]]; then
    profile="debug"
  fi
done

remove_previous_dmgs "${profile}"
cd "${ROOT}/crates/sigillum-desktop"
cargo tauri build "$@"

bundle_root="$(find_bundle_root "${profile}")" || \
  fail "${profile} Sigillum app/dmg bundle root was not produced"
dmg_files=()
while IFS= read -r -d '' candidate; do
  dmg_files+=("${candidate}")
done < <(find "${bundle_root}/dmg" -mindepth 1 -maxdepth 1 \
  -type f -name 'Sigillum_*.dmg' -print0)
if [[ "${#dmg_files[@]}" != "1" ]]; then
  fail "expected exactly one new ${profile} Sigillum dmg; found ${#dmg_files[@]}"
fi

if [[ "${signing_mode}" == "developer-id" ]]; then
  "${ROOT}/scripts/notarize-macos-dmg.sh" "${dmg_files[0]}"
fi
