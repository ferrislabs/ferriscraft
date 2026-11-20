#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "Usage :" >&2
  echo "  $0 <nom-map> <dossier-map>" >&2
  echo "  $0 <nom-map> <world.zip> <arena.json>" >&2
  exit 1
fi

MAP_NAME="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

if [[ $# -eq 2 ]]; then
  MAP_DIR="$2"
  WORLD_ZIP="${MAP_DIR}/world.zip"
  ARENA_JSON="${MAP_DIR}/arena.json"
  if [[ ! -f "${WORLD_ZIP}" && -d "${MAP_DIR}/world" ]]; then
    WORLD_ZIP="$(mktemp -t "${MAP_NAME}-world.XXXXXX.zip")"
    trap 'rm -f "${WORLD_ZIP}"' EXIT
    (cd "${MAP_DIR}" && zip -rq "${WORLD_ZIP}" world)
  fi
else
  WORLD_ZIP="$2"
  ARENA_JSON="$3"
fi

if [[ ! -f "${WORLD_ZIP}" ]]; then
  echo "Erreur : archive introuvable (${WORLD_ZIP})." >&2
  exit 1
fi
if [[ ! -f "${ARENA_JSON}" ]]; then
  echo "Erreur : arena.json introuvable (${ARENA_JSON})." >&2
  exit 1
fi

KUBE_CONTEXT="${KUBE_CONTEXT:-k3d-ferriscraft}"
KUBE_NAMESPACE="${KUBE_NAMESPACE:-ferriscraft}"
S3_BUCKET="${S3_BUCKET:-ferriscraft-maps}"
S3_PREFIX="${S3_PREFIX:-maps/}"
S3_REGION="${S3_REGION:-us-east-1}"
S3_PATH_STYLE="${S3_PATH_STYLE:-true}"

PF_PID=""
cleanup() {
  if [[ -n "${PF_PID}" ]]; then
    kill "${PF_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [[ -z "${S3_ENDPOINT:-}" ]]; then
  LOCAL_PORT="${S3_LOCAL_PORT:-19000}"
  kubectl --context "${KUBE_CONTEXT}" -n "${KUBE_NAMESPACE}" \
    port-forward svc/ferriscraft-rustfs "${LOCAL_PORT}:9000" >/dev/null 2>&1 &
  PF_PID=$!
  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/health/ready" >/dev/null 2>&1; then
      break
    fi
    sleep 0.5
  done
  export S3_ENDPOINT="http://127.0.0.1:${LOCAL_PORT}"
  export S3_ACCESS_KEY_ID="${S3_ACCESS_KEY_ID:-ferriscraft}"
  export S3_SECRET_ACCESS_KEY="${S3_SECRET_ACCESS_KEY:-ferriscraft-dev-s3-secret}"
fi

export S3_BUCKET S3_PREFIX S3_REGION S3_PATH_STYLE

cargo run --manifest-path "${ROOT_DIR}/Cargo.toml" \
  -p ferriscraft-orchestrator \
  --bin ferriscraft-map-sync -- \
  upload "${MAP_NAME}" "${WORLD_ZIP}" "${ARENA_JSON}"
