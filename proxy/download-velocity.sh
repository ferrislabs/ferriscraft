#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
JAR_PATH="${SCRIPT_DIR}/velocity.jar"
SECRET_PATH="${SCRIPT_DIR}/forwarding.secret"
VELOCITY_URL="https://fill-data.papermc.io/v1/objects/0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75/velocity-4.3.1-45.jar"
VELOCITY_SHA256="0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75"

echo "Téléchargement de Velocity 4.3.1 (compatible Minecraft 26.3 / protocole 777)..."
curl -fSL -o "${JAR_PATH}" "${VELOCITY_URL}"
echo "${VELOCITY_SHA256}  ${JAR_PATH}" | shasum -a 256 -c -

if [[ ! -s "${SECRET_PATH}" ]]; then
  openssl rand -hex 24 | tr -d '\n' > "${SECRET_PATH}"
  chmod 600 "${SECRET_PATH}"
fi

SECRET="$(cat "${SECRET_PATH}")"
python3 - "${ROOT_DIR}" "${SECRET}" <<'PY'
import pathlib, re, sys

root = pathlib.Path(sys.argv[1])
secret = sys.argv[2]

for rel in ("server/spigot.yml", "server-sheepwars/spigot.yml"):
    p = root / rel
    example = root / f"{rel}.example"
    if not p.exists() and example.exists():
        p.write_text(example.read_text())
    if not p.exists():
        continue
    text = p.read_text()
    updated = re.sub(
        r"(  velocity:\n\s+enabled:\s*)(?:true|false)(\n\s+secret:\s*).*",
        rf"\g<1>true\g<2>'{secret}'",
        text,
    )
    p.write_text(updated)
PY

echo "Velocity prêt (${JAR_PATH}) et forwarding.secret synchronisé."
