package fr.ferrislabs.ferriscraft.velocity;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.velocitypowered.api.command.SimpleCommand;
import com.velocitypowered.api.event.Subscribe;
import com.velocitypowered.api.event.connection.DisconnectEvent;
import com.velocitypowered.api.event.player.KickedFromServerEvent;
import com.velocitypowered.api.event.player.PlayerChooseInitialServerEvent;
import com.velocitypowered.api.event.proxy.ProxyInitializeEvent;
import com.velocitypowered.api.event.proxy.ProxyShutdownEvent;
import com.velocitypowered.api.plugin.Plugin;
import com.velocitypowered.api.proxy.Player;
import com.velocitypowered.api.proxy.ProxyServer;
import com.velocitypowered.api.proxy.server.RegisteredServer;
import com.velocitypowered.api.proxy.server.ServerInfo;
import net.kyori.adventure.text.Component;
import net.kyori.adventure.text.format.NamedTextColor;
import org.slf4j.Logger;

import com.google.inject.Inject;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

@Plugin(
        id = "ferriscraft-velocity",
        name = "FerrisCraft Velocity Router",
        version = "0.1.0",
        authors = {"FerrisLabs"}
)
public final class FerrisCraftVelocityPlugin {

    private static final Gson GSON = new Gson();

    private final ProxyServer proxy;
    private final Logger logger;
    private final HttpClient http;
    private final String orchestratorUrl;
    private final String apiToken;
    private final Set<String> managedServers = ConcurrentHashMap.newKeySet();
    private final Set<String> lobbyInstances = ConcurrentHashMap.newKeySet();
    private final String version;
    private final Set<String> handledTransfers = ConcurrentHashMap.newKeySet();
    private final AtomicBoolean running = new AtomicBoolean(true);

    private volatile String lobbyServerName = "lobby";
    private Thread sseThread;

    @Inject
    public FerrisCraftVelocityPlugin(ProxyServer proxy, Logger logger) {
        this.proxy = proxy;
        this.logger = logger;
        this.http = HttpClient.newBuilder()
                .connectTimeout(Duration.ofSeconds(5))
                .build();

        String rawUrl = System.getenv("ORCHESTRATOR_URL");
        if (rawUrl == null || rawUrl.isBlank()) {
            rawUrl = "http://127.0.0.1:8080";
        }
        this.orchestratorUrl = rawUrl.replaceAll("/+$", "");

        String rawVersion = System.getenv("FERRISCRAFT_VERSION");
        this.version = (rawVersion == null || rawVersion.isBlank()) ? "dev" : rawVersion.trim();

        String token = System.getenv("ORCHESTRATOR_TOKEN");
        this.apiToken = (token == null || token.isBlank()) ? null : token.trim();
    }

    @Subscribe
    public void onProxyInitialize(ProxyInitializeEvent event) {
        var meta = proxy.getCommandManager()
                .metaBuilder("hub")
                .aliases("lobby")
                .plugin(this)
                .build();
        proxy.getCommandManager().register(meta, new HubCommand());

        proxy.getScheduler()
                .buildTask(this, this::syncStateOnce)
                .repeat(10, TimeUnit.SECONDS)
                .schedule();

        sseThread = Thread.ofVirtual().name("ferriscraft-sse").start(this::runSseLoop);
        logger.info("FerrisCraft Velocity Router démarré (orchestrator={})", orchestratorUrl);
    }

    @Subscribe
    public void onProxyShutdown(ProxyShutdownEvent event) {
        running.set(false);
        if (sseThread != null) {
            sseThread.interrupt();
        }
    }

    @Subscribe
    public void onDisconnect(DisconnectEvent event) {
        UUID uuid = event.getPlayer().getUniqueId();
        JsonObject body = new JsonObject();
        body.addProperty("player_uuid", uuid.toString());
        postJsonAsync("/api/v1/proxy/player-disconnected", body);
    }

    @Subscribe
    public void onChooseInitialServer(PlayerChooseInitialServerEvent event) {
        Optional<RegisteredServer> target = pickLobbyInstance(null).or(() -> proxy.getServer(lobbyServerName));
        target.ifPresent(event::setInitialServer);
    }

    @Subscribe
    public void onKickedFromServer(KickedFromServerEvent event) {
        String kickedFrom = event.getServer().getServerInfo().getName();
        if (kickedFrom.equals(lobbyServerName)) {
            return;
        }
        Optional<RegisteredServer> target = pickLobbyInstance(kickedFrom).or(() -> proxy.getServer(lobbyServerName));
        if (target.isPresent()) {
            Component notice = Component.text("Retour au Hub : le serveur a fermé.", NamedTextColor.YELLOW);
            event.setResult(KickedFromServerEvent.RedirectPlayer.create(target.get(), notice));
        }
    }

    private Optional<RegisteredServer> pickLobbyInstance(String excluded) {
        Map<String, Integer> counts = new HashMap<>();
        for (String name : lobbyInstances) {
            proxy.getServer(name).ifPresent(reg -> counts.put(name, reg.getPlayersConnected().size()));
        }
        return LobbySelector.pick(counts, excluded).flatMap(proxy::getServer);
    }

    private void runSseLoop() {
        while (running.get()) {
            try {
                syncStateOnce();
                streamSse();
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
                break;
            } catch (Exception ex) {
                if (running.get()) {
                    logger.debug("Flux SSE orchestrateur interrompu ({}), reconnexion dans 2s", ex.getMessage());
                }
            }
            if (!running.get()) {
                break;
            }
            try {
                Thread.sleep(2000);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
                break;
            }
        }
    }

    private void streamSse() throws Exception {
        HttpRequest.Builder req = HttpRequest.newBuilder()
                .uri(URI.create(orchestratorUrl + "/api/v1/proxy/events"))
                .header("Accept", "text/event-stream")
                .header("x-ferriscraft-version", version)
                .GET();
        if (apiToken != null) {
            req.header("Authorization", "Bearer " + apiToken);
        }

        HttpResponse<java.io.InputStream> resp = http.send(req.build(), HttpResponse.BodyHandlers.ofInputStream());
        if (resp.statusCode() != 200) {
            throw new IllegalStateException("HTTP " + resp.statusCode());
        }

        try (BufferedReader reader = new BufferedReader(new InputStreamReader(resp.body(), StandardCharsets.UTF_8))) {
            StringBuilder dataBuf = new StringBuilder();
            String line;
            while (running.get() && (line = reader.readLine()) != null) {
                if (line.isEmpty()) {
                    if (!dataBuf.isEmpty()) {
                        handleProxyEventJson(dataBuf.toString());
                        dataBuf.setLength(0);
                    }
                } else if (line.startsWith("data:")) {
                    String part = line.substring(5).trim();
                    if (!dataBuf.isEmpty()) {
                        dataBuf.append('\n');
                    }
                    dataBuf.append(part);
                }
            }
        }
    }

    private void syncStateOnce() {
        try {
            HttpRequest.Builder req = HttpRequest.newBuilder()
                    .uri(URI.create(orchestratorUrl + "/api/v1/proxy/state"))
                    .timeout(Duration.ofSeconds(4))
                    .header("x-ferriscraft-version", version)
                    .GET();
            if (apiToken != null) {
                req.header("Authorization", "Bearer " + apiToken);
            }
            HttpResponse<String> resp = http.send(req.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            if (resp.statusCode() != 200) {
                return;
            }
            JsonObject root = GSON.fromJson(resp.body(), JsonObject.class);
            if (root == null) {
                return;
            }
            if (root.has("lobby_server_name") && !root.get("lobby_server_name").isJsonNull()) {
                lobbyServerName = root.get("lobby_server_name").getAsString();
            }

            Map<String, String> desired = new HashMap<>();
            JsonArray servers = root.getAsJsonArray("servers");
            if (servers != null) {
                for (JsonElement el : servers) {
                    JsonObject obj = el.getAsJsonObject();
                    String name = obj.get("name").getAsString();
                    String address = obj.get("address").getAsString();
                    desired.put(name, address);
                    upsertServer(name, address, isLobby(obj));
                }
            }

            Set<String> toRemove = new HashSet<>(managedServers);
            toRemove.removeAll(desired.keySet());
            for (String stale : toRemove) {
                removeServer(stale);
            }

            JsonArray pending = root.getAsJsonArray("pending_transfers");
            if (pending != null) {
                for (JsonElement el : pending) {
                    JsonObject obj = el.getAsJsonObject();
                    executeTransfer(
                            obj.get("transfer_id").getAsString(),
                            obj.get("player_uuid").getAsString(),
                            obj.get("target_server").getAsString(),
                            obj.has("reason") ? obj.get("reason").getAsString() : ""
                    );
                }
            }
        } catch (Exception ex) {
            logger.debug("Sync état orchestrateur ignorée: {}", ex.getMessage());
        }
    }

    private void handleProxyEventJson(String payload) {
        try {
            JsonObject obj = GSON.fromJson(payload, JsonObject.class);
            if (obj == null || !obj.has("type")) {
                return;
            }
            String type = obj.get("type").getAsString();
            switch (type) {
                case "server_upsert" -> upsertServer(
                        obj.get("name").getAsString(),
                        obj.get("address").getAsString(),
                        isLobby(obj)
                );
                case "server_remove" -> removeServer(obj.get("name").getAsString());
                case "transfer_player" -> executeTransfer(
                        obj.get("transfer_id").getAsString(),
                        obj.get("player_uuid").getAsString(),
                        obj.get("target_server").getAsString(),
                        obj.has("reason") ? obj.get("reason").getAsString() : ""
                );
                default -> logger.debug("Événement proxy inconnu: {}", type);
            }
        } catch (Exception ex) {
            logger.warn("Impossible de traiter l'événement SSE {}: {}", payload, ex.getMessage());
        }
    }

    private static boolean isLobby(JsonObject obj) {
        return obj.has("kind") && !obj.get("kind").isJsonNull() && "lobby".equals(obj.get("kind").getAsString());
    }

    private void upsertServer(String name, String rawAddress, boolean lobby) {
        if (name == null || name.isBlank() || rawAddress == null || !rawAddress.contains(":")) {
            return;
        }
        if (lobby) {
            lobbyInstances.add(name);
        } else {
            lobbyInstances.remove(name);
        }
        int idx = rawAddress.lastIndexOf(':');
        String host = rawAddress.substring(0, idx);
        int port;
        try {
            port = Integer.parseInt(rawAddress.substring(idx + 1));
        } catch (NumberFormatException ex) {
            logger.warn("Port invalide pour serveur {}: {}", name, rawAddress);
            return;
        }

        InetSocketAddress targetAddr = new InetSocketAddress(host, port);
        Optional<RegisteredServer> existing = proxy.getServer(name);
        if (existing.isPresent()) {
            InetSocketAddress current = existing.get().getServerInfo().getAddress();
            if (current.getHostString().equals(host) && current.getPort() == port) {
                managedServers.add(name);
                return;
            }
            proxy.unregisterServer(existing.get().getServerInfo());
        }

        proxy.registerServer(new ServerInfo(name, targetAddr));
        managedServers.add(name);
        logger.info("Serveur enregistré dans Velocity: {} -> {}:{}", name, host, port);
    }

    private void removeServer(String name) {
        managedServers.remove(name);
        lobbyInstances.remove(name);
        proxy.getServer(name).ifPresent(reg -> {
            proxy.unregisterServer(reg.getServerInfo());
            logger.info("Serveur désenregistré de Velocity: {}", name);
        });
    }

    private void executeTransfer(String transferId, String playerUuidRaw, String targetServerName, String reason) {
        if (!handledTransfers.add(transferId)) {
            return;
        }
        if (handledTransfers.size() > 2048) {
            handledTransfers.clear();
            handledTransfers.add(transferId);
        }

        UUID playerUuid;
        try {
            playerUuid = UUID.fromString(playerUuidRaw);
        } catch (IllegalArgumentException ex) {
            ackTransfer(transferId, false, "UUID invalide");
            return;
        }

        Optional<Player> playerOpt = proxy.getPlayer(playerUuid);
        if (playerOpt.isEmpty()) {
            ackTransfer(transferId, false, "Joueur déconnecté du proxy");
            return;
        }

        Optional<RegisteredServer> targetOpt = proxy.getServer(targetServerName);
        if (targetOpt.isEmpty()) {
            ackTransfer(transferId, false, "Serveur cible absent: " + targetServerName);
            return;
        }

        Player player = playerOpt.get();
        RegisteredServer target = targetOpt.get();
        String currentServer = player.getCurrentServer()
                .map(s -> s.getServerInfo().getName())
                .orElse("");
        if (currentServer.equals(targetServerName)) {
            ackTransfer(transferId, true, "Déjà connecté");
            return;
        }

        player.sendMessage(Component.text("Transfert vers ", NamedTextColor.GRAY)
                .append(Component.text(targetServerName, NamedTextColor.LIGHT_PURPLE))
                .append(Component.text(" (" + reason + ")…", NamedTextColor.GRAY)));

        player.createConnectionRequest(target).connectWithIndication().whenComplete((ok, err) -> {
            if (err != null) {
                logger.warn("Erreur transfert {} -> {}: {}", player.getUsername(), targetServerName, err.getMessage());
                ackTransfer(transferId, false, err.getMessage());
            } else {
                boolean success = Boolean.TRUE.equals(ok);
                ackTransfer(transferId, success, success ? null : "Connexion refusée par " + targetServerName);
            }
        });
    }

    private void ackTransfer(String transferId, boolean success, String message) {
        JsonObject body = new JsonObject();
        body.addProperty("success", success);
        if (message != null && !message.isBlank()) {
            body.addProperty("message", message);
        }
        postJsonAsync("/api/v1/proxy/transfers/" + transferId + "/ack", body);
    }

    private void postJsonAsync(String path, JsonObject body) {
        HttpRequest.Builder req = HttpRequest.newBuilder()
                .uri(URI.create(orchestratorUrl + path))
                .timeout(Duration.ofSeconds(4))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(GSON.toJson(body), StandardCharsets.UTF_8));
        if (apiToken != null) {
            req.header("Authorization", "Bearer " + apiToken);
        }
        http.sendAsync(req.build(), HttpResponse.BodyHandlers.discarding())
                .exceptionally(ex -> {
                    logger.debug("POST {} échoué: {}", path, ex.getMessage());
                    return null;
                });
    }

    private final class HubCommand implements SimpleCommand {
        @Override
        public void execute(Invocation invocation) {
            if (!(invocation.source() instanceof Player player)) {
                invocation.source().sendMessage(Component.text("Commande réservée aux joueurs.", NamedTextColor.RED));
                return;
            }
            Optional<RegisteredServer> lobby = proxy.getServer(lobbyServerName);
            if (lobby.isEmpty()) {
                player.sendMessage(Component.text("Le serveur Hub est momentanément indisponible.", NamedTextColor.RED));
                return;
            }
            String current = player.getCurrentServer()
                    .map(s -> s.getServerInfo().getName())
                    .orElse("");
            if (current.equals(lobbyServerName)) {
                player.sendMessage(Component.text("Vous êtes déjà sur le Hub.", NamedTextColor.YELLOW));
                return;
            }
            player.createConnectionRequest(lobby.get()).fireAndForget();
        }
    }
}
