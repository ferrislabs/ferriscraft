#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

REGISTRY="${IMAGE_REGISTRY:-ghcr.io/ferrislabs}"
TAG="${IMAGE_TAG:-dev}"
RSPIGOT_DIR="${RSPIGOT_DIR:-/opt/minecraft/dev/rspigot}"
PUSH="${PUSH_IMAGES:-false}"
K3D_CLUSTER="${K3D_CLUSTER:-}"

if [[ ! -d "${RSPIGOT_DIR}" ]]; then
  echo "Erreur : RSPIGOT_DIR introuvable (${RSPIGOT_DIR}). Définissez RSPIGOT_DIR vers votre clone de ferrislabs/rspigot." >&2
  exit 1
fi

build_image() {
  local name="$1"
  local dockerfile="$2"
  shift 2
  local ref="${REGISTRY}/${name}:${TAG}"
  echo "=== Construction de ${ref} ==="
  docker build \
    -f "${ROOT_DIR}/deploy/docker/${dockerfile}" \
    -t "${ref}" \
    "$@" \
    "${ROOT_DIR}"

  if [[ -n "${K3D_CLUSTER}" ]]; then
    echo "=== Import dans k3d (${K3D_CLUSTER}) : ${ref} ==="
    k3d image import -c "${K3D_CLUSTER}" "${ref}"
  fi

  if [[ "${PUSH}" == "true" ]]; then
    echo "=== Push : ${ref} ==="
    docker push "${ref}"
  fi
}

build_image "ferriscraft-orchestrator" "Dockerfile.orchestrator"
build_image "ferriscraft-velocity" "Dockerfile.velocity"
build_image "ferriscraft-rspigot" "Dockerfile.rspigot" --build-context "rspigot=${RSPIGOT_DIR}"
build_image "ferriscraft-lobby" "Dockerfile.lobby" --build-context "rspigot=${RSPIGOT_DIR}"

echo "Images prêtes (${REGISTRY}/ferriscraft-*:${TAG})."
