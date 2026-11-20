# 0002. Le Lobby tourne en plusieurs instances, mises à jour une par une avec drain

Status: proposed
Date: 2026-11-20

## Context

Le Lobby est un Deployment à un seul Pod qui contient rspigot et le plugin. Velocity l'atteint via un Service unique. Une nouvelle image, de rspigot ou du plugin, recrée tout le Pod et déconnecte tous les joueurs.

L'orchestrateur et le plugin Velocity savent déjà enregistrer des serveurs à la volée (`ServerUpsert`, `ServerRemove`) et déplacer un joueur sans le déconnecter (`TransferPlayer`). C'est le mécanisme utilisé pour les Pods `sheepwars-*`.

## Decision

Le Lobby tourne avec au moins 2 instances. Chaque instance envoie un heartbeat à l'orchestrateur, qui l'annonce à Velocity sous son nom de Pod. Velocity choisit le lobby initial et le lobby de repli parmi les instances `Ready`. Avant l'arrêt d'un Pod, un `preStop` sur rspigot demande un drain à l'orchestrateur : l'instance passe en `Draining`, sort de Velocity, et ses joueurs sont transférés vers une autre instance. Le plugin tourne en sidecar natif pour rester actif jusqu'à la fin du drain. Le rolling update se fait avec `maxUnavailable: 0` et un PDB `minAvailable: 1`.

## Alternatives rejected

- Plugin dans un Deployment séparé, remplacé à chaud via `ToPlugin::Replaced` : garde les joueurs pendant une mise à jour du plugin, mais pas pendant une mise à jour de rspigot. Le gRPC doit sortir de `127.0.0.1`, avec un token. L'état en mémoire du plugin (menus ouverts, files d'attente locales) est perdu quand même.
- Une seule instance et une fenêtre de maintenance : rien à construire, mais chaque correctif coupe tous les joueurs, et un crash du Pod n'a pas de repli.

## Consequences

- Gains : un seul mécanisme couvre les mises à jour du plugin, de rspigot et le déplacement d'un Pod sur un autre nœud. Un crash d'instance a un repli.
- Costs : 2 fois les ressources du Lobby, un endpoint `drain` dans l'orchestrateur, une modification du plugin Velocity, un `preStop` qui dépend de l'orchestrateur.
- What becomes harder : le Lobby dépend de l'orchestrateur pour le routage. Si l'orchestrateur est indisponible, Velocity retombe sur le Service `*-lobby` sans tenir compte du drain. Deux joueurs peuvent être sur deux instances différentes du lobby et ne pas se voir.
