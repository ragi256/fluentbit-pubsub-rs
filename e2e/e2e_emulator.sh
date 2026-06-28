#!/usr/bin/env bash
set -euo pipefail

COMPOSE_FILE="e2e/docker-compose.yml"
PROJECT_ID="e2e-project"
TOPIC="e2e-topic"
SUBSCRIPTION="e2e-sub"
EXPECTED_TEXT="hello-from-emulator-e2e"
CONTAINER_CLI="${CONTAINER_CLI:-docker}"
E2E_FAILED=1

compose() {
  "${CONTAINER_CLI}" compose -f "${COMPOSE_FILE}" "$@"
}

cleanup() {
  if [[ "${E2E_FAILED}" -ne 0 ]]; then
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
  PUBSUB_EMULATOR_HOST="127.0.0.1:8681" python3 scripts/pubsub_emulator_api.py "$@"
}

echo "[1/6] Build Linux shared library for Fluent Bit plugin"
compose run --rm rust-builder cargo build --release --target-dir target/e2e-linux
if [[ ! -f target/e2e-linux/release/libfluent_bit_pubsub_rs.so ]]; then
  echo "E2E failed: plugin shared library was not generated" >&2
  exit 1
fi

echo "[2/6] Start Pub/Sub emulator"
compose up -d pubsub-emulator

echo "Wait for emulator to become ready"
READY=0
for _ in $(seq 1 20); do
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

echo "[3/6] Create topic and subscription in emulator"
run_emulator_api create-topic "${PROJECT_ID}" "${TOPIC}"
run_emulator_api create-subscription "${PROJECT_ID}" "${SUBSCRIPTION}" "${TOPIC}"

echo "[4/6] Run Fluent Bit with plugin"
compose up -d fluent-bit
sleep 8

echo "[5/6] Pull one message from emulator subscription"
RAW_BASE64=""
for _ in $(seq 1 20); do
  RAW_BASE64=$(run_emulator_api pull "${PROJECT_ID}" "${SUBSCRIPTION}" | tr -d '\r\n')
  if [[ -n "${RAW_BASE64}" ]]; then
    break
  fi
  sleep 1
done

if [[ -z "${RAW_BASE64}" ]]; then
  echo "E2E failed: no message was received from emulator" >&2
  exit 1
fi

DECODED=$(printf '%s' "${RAW_BASE64}" | base64 -d 2>/dev/null || true)
if [[ "${DECODED}" != *"${EXPECTED_TEXT}"* ]]; then
  echo "E2E failed: message payload mismatch" >&2
  echo "Decoded payload: ${DECODED}" >&2
  exit 1
fi

echo "[6/6] E2E succeeded"
echo "Decoded payload: ${DECODED}"
E2E_FAILED=0
