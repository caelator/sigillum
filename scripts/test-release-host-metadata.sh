#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COLLECTOR="${ROOT}/scripts/release-host-metadata.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-host-test.XXXXXX")"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "release host metadata test failed: $*" >&2
  exit 1
}

FAKE_BIN="${TMP_ROOT}/bin"
mkdir -p "${FAKE_BIN}"

cat > "${FAKE_BIN}/uname" <<'SH'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf '%s\n' Darwin ;;
  -m) printf '%s\n' arm64 ;;
  -r) printf '%s\n' 25.5.0 ;;
  *) exit 64 ;;
esac
SH
cat > "${FAKE_BIN}/hostname" <<'SH'
#!/usr/bin/env bash
printf '%s\n' mac-server
SH
cat > "${FAKE_BIN}/sw_vers" <<'SH'
#!/usr/bin/env bash
[[ "${1:-}" == "-productVersion" ]] || exit 64
printf '%s\n' 26.5.2
SH
cat > "${FAKE_BIN}/ioreg" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '    "IOPlatformUUID" = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"'
SH
chmod +x \
  "${FAKE_BIN}/uname" \
  "${FAKE_BIN}/hostname" \
  "${FAKE_BIN}/sw_vers" \
  "${FAKE_BIN}/ioreg"

first="$(
  PATH="${FAKE_BIN}:${PATH}" bash "${COLLECTOR}"
)"
second="$(
  PATH="${FAKE_BIN}:${PATH}" bash "${COLLECTOR}"
)"

jq -e '
  .name == "mac-server" and
  .platform == "macos" and
  .product_version == "26.5.2" and
  .arch == "aarch64" and
  (.identity_sha256 |
    type == "string" and test("^[0-9a-f]{64}$"))
' <<< "${first}" >/dev/null ||
  fail "collector did not emit the schema-v2 macOS host fields"

first_identity="$(jq -r '.identity_sha256' <<< "${first}")"
second_identity="$(jq -r '.identity_sha256' <<< "${second}")"
[[ "${first_identity}" == "${second_identity}" ]] ||
  fail "collector did not preserve the same opaque machine identity"

expected_identity="$(
  printf '%s' 'AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE' |
    shasum -a 256 |
    awk '{ print $1 }'
)"
[[ "${first_identity}" == "${expected_identity}" ]] ||
  fail "collector did not hash the fixture hardware identity exactly"

if grep -F 'AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE' <<< "${first}" >/dev/null; then
  fail "collector exposed the raw hardware identity"
fi

echo "release host metadata tests passed"
