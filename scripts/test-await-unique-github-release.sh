#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOOKUP="${ROOT}/scripts/await-unique-github-release.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-release-lookup-test.XXXXXX")"
FAKE_BIN="${TMP_ROOT}/bin"
COUNT_FILE="${TMP_ROOT}/gh-count"
SLEEP_COUNT_FILE="${TMP_ROOT}/sleep-count"

cleanup() {
  rm -rf "${TMP_ROOT}"
}
trap cleanup EXIT

fail() {
  echo "GitHub release lookup test failed: $*" >&2
  exit 1
}

expect_failure() {
  local case_name="$1"
  local expected_message="$2"
  shift 2
  local log_path="${TMP_ROOT}/${case_name}.log"
  if "$@" >"${log_path}" 2>&1; then
    fail "${case_name} unexpectedly passed"
  fi
  grep -F "${expected_message}" "${log_path}" >/dev/null || {
    sed -n '1,120p' "${log_path}" >&2
    fail "${case_name} did not report: ${expected_message}"
  }
}

mkdir -p "${FAKE_BIN}"
cat >"${FAKE_BIN}/gh" <<'FAKE_GH'
#!/usr/bin/env bash
set -euo pipefail

[[ "${1:-}" == "api" ]]
[[ "${2:-}" == "--paginate" ]]
[[ "${3:-}" == "repos/test-owner/test-repo/releases?per_page=100" ]]

count=0
if [[ -s "${SIGILLUM_TEST_GH_COUNT_FILE}" ]]; then
  read -r count < "${SIGILLUM_TEST_GH_COUNT_FILE}"
fi
count=$((count + 1))
printf '%s\n' "${count}" > "${SIGILLUM_TEST_GH_COUNT_FILE}"

case "${SIGILLUM_TEST_GH_SCENARIO}:${count}" in
  delayed:1)
    printf '[]\n'
    ;;
  delayed:*)
    printf '%s\n' \
      '[{"id":41,"tag_name":"v1.0.0-rc.6","draft":true}]' \
      '[{"id":42,"tag_name":"v1.0.0-rc.7","draft":true}]'
    ;;
  duplicate:*)
    printf '%s\n' \
      '[{"id":42,"tag_name":"v1.0.0-rc.7"}]' \
      '[{"id":43,"tag_name":"v1.0.0-rc.7"}]'
    ;;
  absent:*)
    printf '[]\n'
    ;;
  api-error:*)
    exit 1
    ;;
  partial-api-error:*)
    printf '[]\n'
    exit 1
    ;;
  malformed-json:*)
    printf '{\n'
    ;;
  wrong-shape:*)
    printf '{}\n'
    ;;
  missing-tag-name:*)
    printf '[{"id":42}]\n'
    ;;
  empty-output:*)
    :
    ;;
  *)
    exit 2
    ;;
esac
FAKE_GH
chmod +x "${FAKE_BIN}/gh"
cat >"${FAKE_BIN}/sleep" <<'FAKE_SLEEP'
#!/usr/bin/env bash
set -euo pipefail

[[ "${1:-}" == "0" ]]
count=0
if [[ -s "${SIGILLUM_TEST_SLEEP_COUNT_FILE}" ]]; then
  read -r count < "${SIGILLUM_TEST_SLEEP_COUNT_FILE}"
fi
printf '%s\n' "$((count + 1))" > "${SIGILLUM_TEST_SLEEP_COUNT_FILE}"
FAKE_SLEEP
chmod +x "${FAKE_BIN}/sleep"

run_lookup() {
  PATH="${FAKE_BIN}:${PATH}" \
    SIGILLUM_TEST_GH_COUNT_FILE="${COUNT_FILE}" \
    SIGILLUM_TEST_SLEEP_COUNT_FILE="${SLEEP_COUNT_FILE}" \
    SIGILLUM_RELEASE_LOOKUP_ATTEMPTS=3 \
    SIGILLUM_RELEASE_LOOKUP_DELAY_SECONDS=0 \
    bash "${LOOKUP}" test-owner/test-repo v1.0.0-rc.7
}

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=delayed
export SIGILLUM_TEST_GH_SCENARIO
release_json="$(run_lookup)"
jq -e '
  .id == 42 and
  .tag_name == "v1.0.0-rc.7" and
  .draft == true
' <<< "${release_json}" >/dev/null ||
  fail "delayed visibility did not return the unique release"
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 2 ]] ||
  fail "delayed visibility did not retry exactly once"
[[ "$(sed -n '1p' "${SLEEP_COUNT_FILE}")" == 1 ]] ||
  fail "delayed visibility did not sleep exactly once"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=duplicate
expect_failure \
  duplicate \
  "expected exactly one release for v1.0.0-rc.7, found 2" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "duplicate releases must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "duplicate releases must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=absent
expect_failure \
  absent \
  "release v1.0.0-rc.7 was not visible after 3 attempts" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 3 ]] ||
  fail "absent release did not exhaust the bounded retry count"
[[ "$(sed -n '1p' "${SLEEP_COUNT_FILE}")" == 2 ]] ||
  fail "absent release did not sleep only between attempts"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=api-error
expect_failure \
  api-error \
  "could not list releases for test-owner/test-repo" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "API errors must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "API errors must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=partial-api-error
expect_failure \
  partial-api-error \
  "could not list releases for test-owner/test-repo" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "partial API errors must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "partial API errors must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=malformed-json
expect_failure \
  malformed-json \
  "release list response was not valid paginated JSON" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "malformed JSON must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "malformed JSON must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=wrong-shape
expect_failure \
  wrong-shape \
  "release list response was not valid paginated JSON" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "invalid response shapes must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "invalid response shapes must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=missing-tag-name
expect_failure \
  missing-tag-name \
  "release list response was not valid paginated JSON" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "invalid release items must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "invalid release items must fail without sleeping"

rm -f "${COUNT_FILE}"
rm -f "${SLEEP_COUNT_FILE}"
SIGILLUM_TEST_GH_SCENARIO=empty-output
expect_failure \
  empty-output \
  "release list response was not valid paginated JSON" \
  run_lookup
[[ "$(sed -n '1p' "${COUNT_FILE}")" == 1 ]] ||
  fail "empty API output must fail without retrying"
[[ ! -e "${SLEEP_COUNT_FILE}" ]] ||
  fail "empty API output must fail without sleeping"

echo "GitHub release lookup tests passed"
