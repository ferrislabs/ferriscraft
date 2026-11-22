# 0003. Le SDK rspigot est référencé par un tag git

Status: superseded by 0010
Date: 2026-11-20

## Context

`Cargo.toml` pointe les crates `rspigot-*` sur un chemin local (`/opt/minecraft/dev/rspigot`). La CI clone la branche par défaut de `ferrislabs/rspigot` au moment du build. Deux builds du même commit FerrisCraft peuvent donc embarquer deux versions de rspigot, et on ne peut pas savoir laquelle tourne en prod. L'image serveur rspigot et le SDK des plugins doivent pourtant parler le même protocole gRPC.

## Decision

Les dépendances `rspigot-*` sont déclarées avec `git = "https://github.com/ferrislabs/rspigot"` et `tag = "vX.Y.Z"`. La CI construit l'image rspigot à partir du même tag. Monter rspigot se fait par une PR qui change le tag et met à jour `Cargo.lock`. Le dev local continue de pointer sur le chemin local via un `[patch]` dans `.cargo/config.toml`, hors Git.

## Alternatives rejected

- Registre cargo privé : versions propres, mais un service de plus à héberger et à sécuriser pour un seul consommateur.
- `rev` git (sha) : fige aussi la version, mais ne dit pas quelle release on utilise et pousse à dépendre de commits non publiés.
- Garder le chemin local : builds non reproductibles, impossible de revenir en arrière de façon fiable.

## Consequences

- Gains : builds reproductibles, version rspigot lisible dans `Cargo.lock`, retour arrière possible, montées proposables par Renovate.
- Costs : rspigot doit publier des tags. Un correctif rspigot demande un tag puis une PR ici.
- What becomes harder : tester une modification rspigot non publiée demande le `[patch]` local, pas un simple rebuild.
