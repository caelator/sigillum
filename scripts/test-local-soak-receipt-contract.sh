#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOAK="${ROOT}/scripts/check-local-soak.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-soak-receipt-test.XXXXXX")"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "local soak receipt contract test failed: $*" >&2
  exit 1
}

FAKE_BIN="${TMP_ROOT}/bin"
FAKE_VALUE_FILE="${TMP_ROOT}/soak-value"
mkdir -p "${FAKE_BIN}"

cat > "${FAKE_BIN}/cargo" <<'SH'
#!/usr/bin/env bash
case " $* " in
  *" daemon "* | *" -p sigillum-gateway "*)
    trap 'exit 0' TERM INT
    while :; do
      sleep 1
    done
    ;;
  *)
    exit 0
    ;;
esac
SH

cat > "${FAKE_BIN}/curl" <<'SH'
#!/usr/bin/env bash
url="${*: -1}"
body=""
previous=""
for argument in "$@"; do
  if [[ "${previous}" == "--data" ]]; then
    body="${argument}"
    break
  fi
  previous="${argument}"
done

case "${url}" in
  */api/compartment/init)
    printf '%s\n' \
      '{"status":"initialized","compartment_id":0,"session_token":"fixture-token"}'
    ;;
  */api/unlock)
    printf '%s\n' '{"session_token":"fixture-token"}'
    ;;
  */api/status)
    printf '%s\n' \
      '{"initialized":true,"locked":false,"unlocked_compartments":[0],"active_compartment":{"compartment_id":0}}'
    ;;
  */api/api-keys/set)
    value="$(sed -n 's/.*"value":"\([^"]*\)".*/\1/p' <<< "${body}")"
    printf '%s' "${value}" > "${FAKE_SOAK_VALUE_FILE}"
    printf '%s\n' '{"status":"ok","key":"soak_probe"}'
    ;;
  */api/api-keys/get)
    value="$(cat "${FAKE_SOAK_VALUE_FILE}")"
    printf '{"key":"soak_probe","value":"%s"}\n' "${value}"
    ;;
  */api/v1/health)
    printf '%s\n' \
      '{"gateway":"ok","daemon":"ok","daemon_healthy":true}'
    ;;
  *)
    printf '%s\n' '{}'
    ;;
esac
SH
chmod +x "${FAKE_BIN}/cargo" "${FAKE_BIN}/curl"

run_soak() {
  local receipt_path="$1"
  PATH="${FAKE_BIN}:${PATH}" \
  FAKE_SOAK_VALUE_FILE="${FAKE_VALUE_FILE}" \
  SIGILLUM_SOAK_SECONDS=1 \
  SIGILLUM_SOAK_INTERVAL_SECONDS=1 \
  SIGILLUM_SOAK_DAEMON_PORT=29843 \
  SIGILLUM_SOAK_GATEWAY_PORT=29844 \
  SIGILLUM_SOAK_RECEIPT="${receipt_path}" \
    bash "${SOAK}"
}

VALID_RECEIPT="${TMP_ROOT}/valid/receipt.json"
run_soak "${VALID_RECEIPT}" >"${TMP_ROOT}/valid.log" 2>&1 ||
  fail "hermetic passed soak did not persist its receipt"

jq -e '
  .schema_version == 2 and
  .kind == "sigillum.local_soak" and
  .status == "passed" and
  (.host.platform | type == "string" and length > 0) and
  (.host.product_version | type == "string" and length > 0) and
  (.host.arch | type == "string" and length > 0) and
  (.host.identity_sha256 | test("^[0-9a-f]{64}$")) and
  (.evidence.iterations | type == "number" and . > 0) and
  (.evidence.doctor_runs | type == "number" and . > 0)
' "${VALID_RECEIPT}" >/dev/null ||
  fail "hermetic passed soak receipt is not schema v2"

BLOCKED_PARENT="${TMP_ROOT}/not-a-directory"
printf '%s\n' blocker > "${BLOCKED_PARENT}"
if run_soak "${BLOCKED_PARENT}/receipt.json" \
  >"${TMP_ROOT}/blocked.log" 2>&1; then
  fail "passed soak ignored an unwritable receipt path"
fi
grep -F "could not create receipt directory" \
  "${TMP_ROOT}/blocked.log" >/dev/null ||
  fail "passed soak did not report its receipt persistence failure"
[[ -s "${FAKE_VALUE_FILE}" ]] ||
  fail "receipt failure case did not reach a completed soak iteration"

echo "local soak receipt contract tests passed"
