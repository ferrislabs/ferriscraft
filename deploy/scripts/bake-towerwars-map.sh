#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

MAP_NAME="${1:-imperial}"
OUT_DIR="${2:-${ROOT_DIR}/dist/towerwars-maps/${MAP_NAME}}"
RSPIGOT_BIN="${RSPIGOT_BIN:-/opt/minecraft/dev/rspigot/target/release/rspigot}"
SERVER_PORT="${BAKE_SERVER_PORT:-25598}"
GRPC_PORT="${BAKE_GRPC_PORT:-25599}"
BUILD_TIMEOUT_SECS="${BAKE_TIMEOUT_SECS:-240}"

if [[ ! -x "${RSPIGOT_BIN}" ]]; then
  echo "Erreur : binaire rspigot introuvable (${RSPIGOT_BIN}). Définir RSPIGOT_BIN." >&2
  exit 1
fi
for tool in zip mkfifo; do
  command -v "${tool}" >/dev/null || { echo "Erreur : ${tool} requis." >&2; exit 1; }
done

WORK_DIR="$(mktemp -d -t "towerwars-bake-${MAP_NAME}.XXXXXX")"
RSPIGOT_PID=""
PLUGIN_PID=""
cleanup() {
  exec 3>&- 2>/dev/null || true
  [[ -n "${PLUGIN_PID}" ]] && kill "${PLUGIN_PID}" >/dev/null 2>&1 || true
  [[ -n "${RSPIGOT_PID}" ]] && kill "${RSPIGOT_PID}" >/dev/null 2>&1 || true
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

echo "==> Compilation du plugin TowerWars"
cargo build --release --manifest-path "${ROOT_DIR}/Cargo.toml" -p ferriscraft-towerwars \
  --bin ferriscraft-towerwars --example export_map

mkdir -p "${OUT_DIR}"
rm -f "${OUT_DIR}/world.zip" "${OUT_DIR}/arena.json"
"${ROOT_DIR}/target/release/examples/export_map" > "${OUT_DIR}/arena.json"

echo "==> Préparation du serveur temporaire (${WORK_DIR})"
for file in server.properties bukkit.yml permissions.yml plugins.yml; do
  cp "${ROOT_DIR}/server-towerwars/${file}" "${WORK_DIR}/${file}"
done
sed -i.bak -e "s/^server-port=.*/server-port=${SERVER_PORT}/" "${WORK_DIR}/server.properties"
sed -i.bak \
  -e "s/^bind: .*/bind: 127.0.0.1:${GRPC_PORT}/" \
  -e "s/^port: .*/port: ${GRPC_PORT}/" \
  "${WORK_DIR}/plugins.yml"
rm -f "${WORK_DIR}"/*.bak

mkfifo "${WORK_DIR}/console"
"${RSPIGOT_BIN}" --world-dir "${WORK_DIR}" < "${WORK_DIR}/console" > "${WORK_DIR}/rspigot.log" 2>&1 &
RSPIGOT_PID=$!
exec 3> "${WORK_DIR}/console"

for _ in $(seq 1 60); do
  grep -q "Starting plugin host" "${WORK_DIR}/rspigot.log" && break
  sleep 0.5
done

echo "==> Construction de la carte par le plugin"
RUST_LOG=info \
RSPIGOT_URL="http://127.0.0.1:${GRPC_PORT}" \
RSPIGOT_PLUGIN_NAME=ferriscraft-towerwars \
TOWERWARS_MAP_PATH="${OUT_DIR}/arena.json" \
  "${ROOT_DIR}/target/release/ferriscraft-towerwars" > "${WORK_DIR}/plugin.log" 2>&1 &
PLUGIN_PID=$!

deadline=$((SECONDS + BUILD_TIMEOUT_SECS))
until grep -q "arène construite" "${WORK_DIR}/plugin.log"; do
  if (( SECONDS > deadline )); then
    echo "Erreur : carte non construite après ${BUILD_TIMEOUT_SECS} s." >&2
    tail -20 "${WORK_DIR}/plugin.log" >&2
    exit 1
  fi
  sleep 1
done
built_line="$(grep "arène construite" "${WORK_DIR}/plugin.log" | sed 's/\x1b\[[0-9;]*m//g')"
echo "    ${built_line##*engine: }"
if ! grep -q "pending_chunks=0" <<< "${built_line}"; then
  echo "Erreur : des chunks n'ont pas été construits." >&2
  exit 1
fi

sleep 3
echo "==> Sauvegarde et arrêt du serveur"
echo "save-all flush" >&3
sleep 3
echo "stop" >&3
for _ in $(seq 1 120); do
  kill -0 "${RSPIGOT_PID}" 2>/dev/null || break
  sleep 0.5
done
if kill -0 "${RSPIGOT_PID}" 2>/dev/null; then
  echo "Erreur : rspigot ne s'est pas arrêté." >&2
  exit 1
fi
RSPIGOT_PID=""
kill "${PLUGIN_PID}" >/dev/null 2>&1 || true
PLUGIN_PID=""

echo "==> Archivage du monde"
(cd "${WORK_DIR}" && zip -rq "${OUT_DIR}/world.zip" world \
  -x 'world/playerdata/*' 'world/stats/*' 'world/advancements/*' 'world/session.lock')

echo
echo "Carte prête dans ${OUT_DIR} :"
ls -lh "${OUT_DIR}"
echo
echo "Téléverser sur RustFS (préfixe TowerWars, jamais celui de SheepWars) :"
echo "  S3_PREFIX=towerwars/ ./deploy/scripts/upload-map.sh ${MAP_NAME} ${OUT_DIR#"${ROOT_DIR}/"}"
echo "Puis activer : --set orchestrator.towerwars.map=${MAP_NAME}"
