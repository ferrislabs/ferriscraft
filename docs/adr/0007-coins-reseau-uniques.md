# 0007. Une seule monnaie de coins pour tout le réseau

Status: proposed
Date: 2026-11-22

## Context

Les levels sont tenus par mini-jeu et au niveau du réseau. Pour la monnaie, deux modèles existent : un solde par mini-jeu (comme Hypixel) ou un solde unique.

## Decision

Un seul solde de coins par joueur, gagné dans tous les mini-jeux et dépensé dans une boutique commune. Chaque mouvement est une ligne de journal avec sa raison. Le solde est tenu dans la même transaction et ne peut pas devenir négatif.

## Alternatives rejected

- Coins par mini-jeu : chaque jeu garde son économie, mais il faut un solde, un journal et une boutique par jeu, et les cosmétiques du lobby n'ont pas de monnaie naturelle.

## Consequences

- Gains : un modèle de données simple, un affichage unique, des cosmétiques achetables avec n'importe quel jeu.
- Costs : un mini-jeu très rentable déséquilibre toute l'économie. Les gains par jeu doivent être réglés ensemble.
- What becomes harder : passer plus tard à des coins par jeu demandera de répartir les soldes existants.
