# 0006. La progression est rattachée à l'UUID Minecraft

Status: proposed
Date: 2026-11-22

## Context

Velocity tourne en online-mode : chaque joueur a un UUID Minecraft vérifié par Mojang. Le compte FerrisKey n'existe que si le joueur fait `/login`, et sert aujourd'hui au grade et à Capitainerie.

## Decision

Toutes les données de progression ont pour clé l'UUID Minecraft. Le grade FerrisKey est un attribut du joueur, envoyé par le lobby à chaque arrivée et à chaque `/login`, et mis en cache par l'orchestrateur.

## Alternatives rejected

- Clé sur le `sub` FerrisKey : la progression suivrait le compte Ferrislabs, mais un joueur sans `/login` ne gagnerait rien. Trop de friction pour une première partie.

## Consequences

- Gains : tout joueur progresse dès sa première partie, sans étape préalable.
- Costs : le grade est un cache. Un grade retiré dans FerrisKey reste actif jusqu'au prochain passage du joueur au lobby.
- What becomes harder : rattacher plus tard la progression à un compte Ferrislabs demandera une table de liaison UUID ↔ `sub`. Un joueur qui change de compte Minecraft repart de zéro.
