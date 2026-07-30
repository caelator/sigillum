#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "release host metadata failed: $*" >&2
  exit 1
}

for command_name in hostname node shasum uname; do
  command -v "${command_name}" >/dev/null 2>&1 ||
    fail "required command is missing: ${command_name}"
done

kernel_name="$(uname -s 2>/dev/null)" ||
  fail "could not determine the host kernel"
raw_arch="$(uname -m 2>/dev/null)" ||
  fail "could not determine the host architecture"
host_name="$(hostname 2>/dev/null)" ||
  fail "could not determine the host name"

case "${raw_arch}" in
  arm64 | aarch64)
    canonical_arch="aarch64"
    ;;
  amd64 | x86_64)
    canonical_arch="x86_64"
    ;;
  *)
    canonical_arch="unknown"
    ;;
esac

case "${kernel_name}" in
  Darwin)
    command -v sw_vers >/dev/null 2>&1 ||
      fail "sw_vers is required to record macOS ProductVersion"
    command -v ioreg >/dev/null 2>&1 ||
      fail "ioreg is required to derive the macOS host identity"
    platform="macos"
    product_version="$(sw_vers -productVersion 2>/dev/null)" ||
      fail "could not read macOS ProductVersion"
    identity_source="$(
      ioreg -rd1 -c IOPlatformExpertDevice 2>/dev/null |
        awk -F'= ' '/"IOPlatformUUID"/ {
          gsub(/[" ]/, "", $2)
          print $2
          exit
        }'
    )"
    [[ "${identity_source}" =~ ^[0-9A-Fa-f-]{36}$ ]] ||
      fail "could not read a valid macOS IOPlatformUUID"
    ;;
  Linux)
    platform="linux"
    product_version="$(uname -r 2>/dev/null)" ||
      fail "could not determine the Linux kernel version"
    if [[ -r /etc/machine-id ]]; then
      identity_source="$(tr -d '[:space:]' < /etc/machine-id)"
    else
      identity_source="${host_name}"
    fi
    [[ -n "${identity_source}" ]] ||
      fail "could not derive a Linux host identity"
    ;;
  *)
    platform="unknown"
    product_version="$(uname -r 2>/dev/null || printf 'unknown')"
    identity_source="${host_name}"
    ;;
esac

identity_sha256="$(
  printf '%s' "${identity_source}" |
    shasum -a 256 |
    awk '{ print $1 }'
)"
[[ "${identity_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
  fail "could not hash the host identity"

# The JavaScript source intentionally reads process.env at runtime.
# shellcheck disable=SC2016
SIGILLUM_HOST_NAME="${host_name}" \
SIGILLUM_HOST_PLATFORM="${platform}" \
SIGILLUM_HOST_PRODUCT_VERSION="${product_version}" \
SIGILLUM_HOST_ARCH="${canonical_arch}" \
SIGILLUM_HOST_IDENTITY_SHA256="${identity_sha256}" \
  node -e '
const data = {
  name: process.env.SIGILLUM_HOST_NAME,
  platform: process.env.SIGILLUM_HOST_PLATFORM,
  product_version: process.env.SIGILLUM_HOST_PRODUCT_VERSION,
  arch: process.env.SIGILLUM_HOST_ARCH,
  identity_sha256: process.env.SIGILLUM_HOST_IDENTITY_SHA256,
};
process.stdout.write(`${JSON.stringify(data)}\n`);
'
