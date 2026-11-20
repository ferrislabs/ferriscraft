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

TARGETS=("$@")
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  TARGETS=("default")
fi

target_image_name() {
  case "$1" in
    orchestrator) echo "ferriscraft-orchestrator" ;;
    velocity) echo "ferriscraft-velocity" ;;
    rspigot) echo "ferriscraft-rspigot" ;;
    lobby) echo "ferriscraft-lobby" ;;
    sheepwars) echo "ferriscraft-sheepwars-plugin" ;;
    *)
      echo "Cible inconnue : $1 (attendu : orchestrator | velocity | rspigot | lobby | sheepwars)" >&2
      exit 1
      ;;
  esac
}

if [[ "${TARGETS[0]}" == "default" ]]; then
  IMAGE_NAMES=("ferriscraft-orchestrator" "ferriscraft-velocity" "ferriscraft-rspigot" "ferriscraft-lobby" "ferriscraft-sheepwars-plugin")
else
  IMAGE_NAMES=()
  for t in "${TARGETS[@]}"; do
    IMAGE_NAMES+=("$(target_image_name "${t}")")
  done
fi

echo "=== BuildKit Bake (${REGISTRY}/*:${TAG}) : ${TARGETS[*]} ==="
(
  cd "${ROOT_DIR}"
  DOCKER_BUILDKIT=1 \
  IMAGE_REGISTRY="${REGISTRY}" \
  IMAGE_TAG="${TAG}" \
  RSPIGOT_DIR="${RSPIGOT_DIR}" \
    docker buildx bake \
      -f "${ROOT_DIR}/docker-bake.hcl" \
      --allow="fs.read=${RSPIGOT_DIR}" \
      --load \
      "${TARGETS[@]}"
)

for name in "${IMAGE_NAMES[@]}"; do
  ref="${REGISTRY}/${name}:${TAG}"
  if [[ -n "${K3D_CLUSTER}" ]]; then
    echo "=== Import dans k3d (${K3D_CLUSTER}) : ${ref} ==="
    k3d image import -c "${K3D_CLUSTER}" "${ref}"
  fi

  if [[ "${PUSH}" == "true" ]]; then
    echo "=== Push : ${ref} ==="
    docker push "${ref}"
  fi
done

echo "Images prêtes (${REGISTRY}/ferriscraft-*:${TAG})."
