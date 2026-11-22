package fr.ferrislabs.ferriscraft.velocity;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Map;
import java.util.Optional;
import org.junit.jupiter.api.Test;

class LobbySelectorTest {

    @Test
    void emptyYieldsNone() {
        assertTrue(LobbySelector.pick(Map.of(), null).isEmpty());
    }

    @Test
    void picksFewestPlayers() {
        Map<String, Integer> lobbies = Map.of("lobby-a", 5, "lobby-b", 1, "lobby-c", 3);
        assertEquals(Optional.of("lobby-b"), LobbySelector.pick(lobbies, null));
    }

    @Test
    void tieBreaksByName() {
        Map<String, Integer> lobbies = Map.of("lobby-b", 2, "lobby-a", 2);
        assertEquals(Optional.of("lobby-a"), LobbySelector.pick(lobbies, null));
    }

    @Test
    void excludesKickingLobby() {
        Map<String, Integer> lobbies = Map.of("lobby-a", 0, "lobby-b", 9);
        assertEquals(Optional.of("lobby-b"), LobbySelector.pick(lobbies, "lobby-a"));
    }

    @Test
    void onlyExcludedYieldsNone() {
        assertTrue(LobbySelector.pick(Map.of("lobby-a", 0), "lobby-a").isEmpty());
    }
}
