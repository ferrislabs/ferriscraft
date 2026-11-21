# Glossaire

| Terme | Définition | Synonyme rejeté |
|---|---|---|
| instance de lobby | Un Pod lobby (rspigot + plugin `ferriscraft-lobby`), annoncé à Velocity sous son nom de Pod. | réplica, hub |
| drain | Passage d'une instance en `Draining` : elle sort de Velocity et ses joueurs sont transférés ailleurs avant l'arrêt du Pod. | évacuation, vidage |
| fenêtre de compatibilité | Période d'une mise à jour où les versions N et N-1 tournent ensemble et doivent se comprendre (JSON, schéma Postgres). | période de transition |
| sidecar natif | Conteneur déclaré dans `initContainers` avec `restartPolicy: Always`. Il démarre avant les conteneurs principaux et s'arrête après eux. | sidecar (sans précision) |
