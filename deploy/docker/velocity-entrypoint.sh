#!/usr/bin/env bash
set -euo pipefail

cd /app

SECRET="${FORWARDING_SECRET:-}"
if [[ -z "${SECRET}" && -f /etc/ferriscraft/forwarding/secret ]]; then
  SECRET="$(tr -d '\r\n' < /etc/ferriscraft/forwarding/secret)"
fi
if [[ -z "${SECRET}" ]]; then
  echo "Erreur : FORWARDING_SECRET ou /etc/ferriscraft/forwarding/secret manquant." >&2
  exit 1
fi
printf '%s' "${SECRET}" > /app/forwarding.secret
chmod 600 /app/forwarding.secret

LOBBY_ADDRESS="${LOBBY_ADDRESS:-ferriscraft-lobby:25565}"
VELOCITY_ONLINE_MODE="${VELOCITY_ONLINE_MODE:-true}"
SHOW_MAX_PLAYERS="${SHOW_MAX_PLAYERS:-100}"
MOTD="${VELOCITY_MOTD:-<gold><bold>FerrisCraft</bold></gold> <dark_gray>•</dark_gray> <light_purple>Mini-Jeux & SheepWars</light_purple>}"

cat > /app/velocity.toml <<EOF
config-version = "2.9"

bind = "0.0.0.0:25565"
motd = "${MOTD}"
show-max-players = ${SHOW_MAX_PLAYERS}

online-mode = ${VELOCITY_ONLINE_MODE}
force-key-authentication = false
prevent-client-proxy-connections = false

player-info-forwarding-mode = "modern"
forwarding-secret-file = "forwarding.secret"

announce-forge = false
kick-existing-players = false
sample-players-in-ping = false
enable-player-address-logging = true

[ping-passthrough]
version = false
players = true
description = false
favicon = true
modinfo = false

[servers]
lobby = "${LOBBY_ADDRESS}"

try = [
    "lobby"
]

[forced-hosts]

[advanced]
compression-threshold = 256
compression-level = -1
login-ratelimit = 500
connection-timeout = 5000
read-timeout = 30000
haproxy-protocol = false
tcp-fast-open = false
bungee-plugin-message-channel = true
show-ping-requests = false
failover-on-unexpected-server-disconnect = true
announce-proxy-commands = true
log-command-executions = false
log-player-connections = true
accepts-transfers = true
enable-reuse-port = false
command-rate-limit = 50
forward-commands-if-rate-limited = true
kick-after-rate-limited-commands = 0
tab-complete-rate-limit = 10
kick-after-rate-limited-tab-completes = 0

[query]
enabled = false
port = 25565
map = "FerrisCraft"
show-plugins = false
EOF

JVM_OPTS="${JAVA_OPTS:--Xms512M -Xmx512M -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200}"
exec java ${JVM_OPTS} -jar /app/velocity.jar "$@"
