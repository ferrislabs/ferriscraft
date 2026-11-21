# 0001. L'état du Lobby est stocké dans Postgres (CloudNativePG)

Status: proposed
Date: 2026-11-20

## Context

Le plugin `ferriscraft-lobby` garde deux états dans des fichiers du conteneur :

- le layout (`/app/lobby.json`), copié dans l'image et modifié en jeu par `/lobby set*` ;
- les sessions FerrisKey et Capitainerie (`/tmp/.sessions.json`), qui contiennent des `refresh_token`.

Les deux sont perdus à chaque redémarrage du Pod. [ADR 0002](0002-plusieurs-lobbies-et-drain.md) fait tourner au moins 2 instances de lobby : l'état doit donc être partagé entre Pods.

## Decision

Le layout et les sessions sont stockés dans une base Postgres gérée par CloudNativePG, déployée par le chart. Le plugin y accède via `sqlx`, derrière deux ports (`LayoutRepository`, `SessionRepository`). L'adapter fichier actuel reste disponible pour le dev local. Les migrations sont lancées par un Job Helm avant la mise à jour, jamais par le plugin. Elles suivent le schéma expand puis contract, pour que les versions N et N-1 du plugin tournent sur le même schéma.

## Alternatives rejected

- Volume persistant (PVC) : un PVC `ReadWriteOnce` ne se monte que sur un nœud à la fois, ce qui bloque le passage à 2 lobbies. `ReadWriteMany` dépend du stockage du cluster et n'offre pas d'écriture concurrente sûre sur un fichier JSON.
- Layout dans le bucket S3 des maps : convient au layout, pas aux sessions. Les écritures concurrentes de sessions deviennent des courses sur des objets.
- Redis : bon pour des sessions, mais un service de plus sans persistance garantie par défaut, et le layout n'est pas un cache.

## Consequences

- Gains : l'état survit aux redémarrages, il est partagé entre instances, on peut le sauvegarder et le restaurer avec les outils CNPG.
- Costs : un cluster Postgres à opérer, `sqlx` comme nouvelle dépendance, un Job de migration dans le chart.
- Sécurité : les `refresh_token` sont stockés en clair en base, protégés seulement par le chiffrement disque du stockage CNPG. Choix accepté le 2026-11-20, à revoir si la base est partagée avec d'autres services.
- What becomes harder : chaque changement de schéma doit rester compatible avec la version précédente du plugin. Le dev local sans Postgres passe par l'adapter fichier, qui doit rester à jour.
