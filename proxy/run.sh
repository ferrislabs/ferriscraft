#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -f "${SCRIPT_DIR}/velocity.jar" ]]; then
  "${SCRIPT_DIR}/download-velocity.sh"
fi

find_java25() {
  local candidates=(
    "${JAVA_HOME:-}/bin/java"
    "/opt/homebrew/opt/openjdk@25/bin/java"
    "/opt/homebrew/opt/openjdk/bin/java"
    "/usr/local/opt/openjdk@25/bin/java"
    "/usr/local/opt/openjdk/bin/java"
    "$(command -v java || true)"
  )
  for bin in "${candidates[@]}"; do
    if [[ -n "${bin}" && -x "${bin}" ]]; then
      local ver
      ver="$("${bin}" -version 2>&1 | head -n 1)"
      if [[ "${ver}" =~ \"([0-9]+) ]] && (( BASH_REMATCH[1] >= 25 )); then
        echo "${bin}"
        return 0
      fi
    fi
  done
  return 1
}

if ! JAVA_BIN="$(find_java25)"; then
  echo "Erreur : Velocity 4.3.x (Minecraft 26.3) requiert Java 25+ (ex: brew install openjdk@25)." >&2
  exit 1
fi

cd "${SCRIPT_DIR}"
exec "${JAVA_BIN}" -Xms512M -Xmx512M -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 -jar velocity.jar "$@"
