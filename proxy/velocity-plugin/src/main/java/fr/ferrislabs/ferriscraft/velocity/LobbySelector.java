package fr.ferrislabs.ferriscraft.velocity;

import java.util.Map;
import java.util.Optional;

final class LobbySelector {

    private LobbySelector() {
    }

    static Optional<String> pick(Map<String, Integer> playersByLobby, String excluded) {
        String best = null;
        int bestCount = Integer.MAX_VALUE;
        for (Map.Entry<String, Integer> entry : playersByLobby.entrySet()) {
            String name = entry.getKey();
            if (name.equals(excluded)) {
                continue;
            }
            int count = entry.getValue();
            if (best == null || count < bestCount || (count == bestCount && name.compareTo(best) < 0)) {
                best = name;
                bestCount = count;
            }
        }
        return Optional.ofNullable(best);
    }
}
