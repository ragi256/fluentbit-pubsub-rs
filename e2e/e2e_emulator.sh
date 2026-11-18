#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.yml"
PROJECT_ID="e2e-project"
TOPIC="e2e-topic"
EXPECTED_PAYLOAD='{"message":"hello-from-emulator-e2e"}'
CONTAINER_CLI="${CONTAINER_CLI:-docker}"
E2E_FAILED=1

compose() {
  "${CONTAINER_CLI}" compose -f "${COMPOSE_FILE}" "$@"
}

cleanup() {
  if [[ "${E2E_FAILED}" -ne 0 ]]; then
    compose ps --all || true
    echo "[debug] Fluent Bit logs"
    compose logs --no-color fluent-bit || true
    echo "[debug] Pub/Sub emulator logs"
    compose logs --no-color pubsub-emulator || true
  fi
  compose down -v --remove-orphans >/dev/null 2>&1 || true
}

trap cleanup EXIT

if ! command -v "${CONTAINER_CLI}" >/dev/null 2>&1; then
  echo "${CONTAINER_CLI} command is required" >&2
  exit 1
fi

run_emulator_api() {
  PUBSUB_EMULATOR_HOST="127.0.0.1:8681" python3 "${SCRIPT_DIR}/pubsub_emulator_api.py" "$@"
}

echo "[1/9] Build native Linux shared libraries for the Fluent Bit plugin and allocator probe"
compose run --rm rust-builder cargo build --locked --release --lib --example allocator_probe \
  --target-dir target/e2e-linux
if [[ ! -f "${REPO_ROOT}/target/e2e-linux/release/libfluent_bit_pubsub_rs.so" ]]; then
  echo "E2E failed: plugin shared library was not generated" >&2
  exit 1
fi

bash "${REPO_ROOT}/scripts/check-glibc.sh" \
  "${REPO_ROOT}/target/e2e-linux/release/libfluent_bit_pubsub_rs.so" GLIBC_2.34

echo "[2/9] Verify allocation alignment inside Fluent Bit"
probe_output=$(compose run --rm --no-deps \
  -v "${REPO_ROOT}/target/e2e-linux/release/examples/liballocator_probe.so:/probe/liballocator_probe.so:ro" \
  fluent-bit /fluent-bit/bin/fluent-bit -e /probe/liballocator_probe.so --dry-run -i dummy -o null 2>&1) \
  || { printf '%s\n' "${probe_output}" >&2; exit 1; }
printf '%s\n' "${probe_output}"
if [[ "${probe_output}" != *"allocator probe passed"* ]]; then
  echo "E2E failed: Fluent Bit did not run the allocator probe" >&2
  exit 1
fi

echo "[3/9] Start Pub/Sub emulator"
compose up -d pubsub-emulator

echo "Wait for emulator to become ready"
READY=0
for _ in $(seq 1 60); do
  if run_emulator_api ping >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 1
done
if [[ "${READY}" -ne 1 ]]; then
  echo "E2E failed: Pub/Sub emulator did not become ready in time" >&2
  exit 1
fi

echo "[4/9] Create smoke test topic in emulator"
run_emulator_api create-topic "${PROJECT_ID}" "${TOPIC}"

verify_message() {
  local subscription=$1
  local expected_payload=$2
  local expected_attributes=$3
  local received_message=""

  for _ in $(seq 1 30); do
    received_message=$(run_emulator_api pull-message "${PROJECT_ID}" "${subscription}")
    if [[ -n "${received_message}" ]]; then
      break
    fi
    sleep 1
  done

  if [[ -z "${received_message}" ]]; then
    echo "E2E failed: no message was received for ${subscription}" >&2
    return 1
  fi

  run_emulator_api validate-message "${received_message}" "${expected_payload}" "${expected_attributes}"
}

verify_no_extra_messages() {
  local subscription received_message

  # Allow several Fluent Bit flush intervals for any unexpected messages to arrive.
  for _ in {1..5}; do
    sleep 1
    for subscription in "$@"; do
      received_message=$(run_emulator_api pull-message "${PROJECT_ID}" "${subscription}")
      if [[ -n "${received_message}" ]]; then
        printf 'E2E failed: unexpected message on %s: %s\n' "${subscription}" "${received_message}" >&2
        return 1
      fi
    done
  done
}

run_case() {
  local subscription=$1
  local config_file=$2
  local emulator_host=$3
  local expected_attributes=$4

  run_emulator_api create-subscription "${PROJECT_ID}" "${subscription}" "${TOPIC}"
  E2E_CONFIG_FILE="${config_file}" EMULATOR_HOST_FOR_CONTAINERS="${emulator_host}" \
    compose up -d --no-deps --force-recreate fluent-bit
  verify_message "${subscription}" "${EXPECTED_PAYLOAD}" "${expected_attributes}"
}

echo "[5/9] Verify default routing and messages without attributes"
run_case e2e-default fluent-bit.conf "${EMULATOR_HOST_FOR_CONTAINERS:-pubsub-emulator:8681}" '{}'

echo "[6/9] Verify attributes and explicit endpoint routing"
# An unusable fallback host proves the plugin actually uses Endpoint, while
# PUBSUB_EMULATOR_HOST keeps the emulator's anonymous authentication enabled.
run_case e2e-options fluent-bit-options.conf unused.invalid:8681 \
  '{"environment":"e2e","source":"firelens","description":"comma,equals=ok"}'

echo "[7/9] Verify independent routing for two Pub/Sub outputs"
for topic in e2e-data e2e-log; do
  run_emulator_api create-topic "${PROJECT_ID}" "${topic}"
  run_emulator_api create-subscription "${PROJECT_ID}" "${topic}" "${topic}"
done
E2E_CONFIG_FILE=fluent-bit-multi-output.conf compose up -d --no-deps --force-recreate fluent-bit
verify_message e2e-data 'message from data' '{"source":"data"}'
verify_message e2e-log 'message from log' '{"source":"log"}'
verify_no_extra_messages e2e-data e2e-log
if [[ -z $(compose ps --status running --quiet fluent-bit) ]]; then
  echo "E2E failed: Fluent Bit exited during the two-output test" >&2
  exit 1
fi
compose stop --timeout 10 fluent-bit
container_id=$(compose ps --all --quiet fluent-bit)
exit_code=$("${CONTAINER_CLI}" inspect --format '{{.State.ExitCode}}' "${container_id}")
if [[ "${exit_code}" -ne 0 ]]; then
  echo "E2E failed: Fluent Bit with two outputs exited with code ${exit_code}" >&2
  exit 1
fi

echo "[8/9] Verify publishing under load"
load_runs=1
if [[ $(uname -m) == x86_64 ]]; then
  load_runs=5
fi
load_topic=e2e-load
run_emulator_api create-topic "${PROJECT_ID}" "${load_topic}"
for ((run = 1; run <= load_runs; run++)); do
  subscription="${load_topic}-${run}"
  echo "Load run ${run}/${load_runs}"
  run_emulator_api create-subscription "${PROJECT_ID}" "${subscription}" "${load_topic}"
  E2E_CONFIG_FILE=fluent-bit-load.conf compose up -d --no-deps --force-recreate fluent-bit

  # With no restart policy, any Fluent Bit exit ends the wait before the deadline.
  wait_status=0
  timeout 40s "${CONTAINER_CLI}" compose -f "${COMPOSE_FILE}" wait fluent-bit || wait_status=$?
  if [[ "${wait_status}" -ne 124 || -z $(compose ps --status running --quiet fluent-bit) ]]; then
    echo "E2E failed: Fluent Bit did not stay running for 40 seconds (wait exit ${wait_status})" >&2
    exit 1
  fi

  run_emulator_api verify-count "${PROJECT_ID}" "${subscription}" 10000
  if [[ -z $(compose ps --status running --quiet fluent-bit) ]]; then
    echo "E2E failed: Fluent Bit exited while verifying message delivery" >&2
    exit 1
  fi
  compose stop --timeout 10 fluent-bit
  run_emulator_api delete-subscription "${PROJECT_ID}" "${subscription}"
done

echo "[9/9] E2E succeeded"
E2E_FAILED=0
