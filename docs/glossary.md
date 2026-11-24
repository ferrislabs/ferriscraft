# Glossaire

| Terme | Définition | Synonyme rejeté |
|---|---|---|
| instance de lobby | Un Pod lobby (rspigot + plugin `ferriscraft-lobby`), annoncé à Velocity sous son nom de Pod. | réplica, hub |
| drain | Passage d'une instance en `Draining` : elle sort de Velocity et ses joueurs sont transférés ailleurs avant l'arrêt du Pod. | évacuation, vidage |
| fenêtre de compatibilité | Période d'une mise à jour où les versions N et N-1 tournent ensemble et doivent se comprendre (JSON, schéma Postgres). | période de transition |
| sidecar natif | Conteneur déclaré dans `initContainers` avec `restartPolicy: Always`. Il démarre avant les conteneurs principaux et s'arrête après eux. | sidecar (sans précision) |
| XP | Points d'expérience gagnés dans une partie comptée, par mini-jeu. | EXP, points |
| level de jeu | Level calculé depuis l'XP d'un mini-jeu avec la courbe `GAME`. | étoile, prestige |
| level réseau | Level calculé depuis la somme des XP de tous les mini-jeux avec la courbe `NETWORK`. | level global |
| coins | Monnaie unique du réseau, gagnée en jouant, dépensée en cosmétiques. | pièces, crédits |
| partie comptée | Manche qui rapporte XP et coins : pas de démarrage forcé, au moins 2 participants, au moins 60 s. | partie classée |
| rapport de partie | Message `MatchReport` envoyé par un mini-jeu à l'orchestrateur en fin de manche. | résultat, score |
| cosmétique | Effet visuel sans effet de gameplay, à débloquer puis équiper dans un emplacement. | skin, bonus |
| déblocage | Droit d'équiper un cosmétique, obtenu par achat, grade ou level réseau. | unlock |
| grade | Rang en jeu (`Admin` à `Guest`) déduit des rôles FerrisKey du joueur. | rôle (réservé à FerrisKey) |
| jeu | Mini-jeu hébergé par le réseau (`GameKind` : SheepWars, TowerWars). Il a sa file, sa flotte, sa politique, ses Pods et son CRD dans l'orchestrateur. | mode, mini-jeu (seul), game (hors code) |
