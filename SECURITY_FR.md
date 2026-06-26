# Politique de sécurité

[🇬🇧 English](SECURITY.md) · 🇫🇷 Français

BetterInstaller installe, met à jour et supprime des logiciels, et vérifie l'intégrité
et la signature des paquets. Nous prenons la sécurité au sérieux et apprécions la
divulgation responsable.

## Signaler une vulnérabilité

**N'ouvre pas d'issue publique pour une vulnérabilité.** Signale-la en privé :

- Utilise le **[signalement privé de vulnérabilité](https://github.com/FreeProject089/BetterInstaller/security/advisories/new)**
  de GitHub (onglet Security → *Report a vulnerability*), **ou**
- Contacte directement le mainteneur (voir le profil de
  [@FreeProject089](https://github.com/FreeProject089)).

Merci d'inclure :

- Une description claire et l'**impact** (ce qu'un attaquant peut obtenir).
- Les **étapes pour reproduire** (un `installer.toml` / paquet / commande minimal est idéal).
- La/les version(s) et l'OS affectés, et tout log ou preuve de concept.

Nous visons un accusé de réception sous **72 h** et un calendrier de correction après
triage. Merci de nous laisser un délai raisonnable pour corriger avant toute
divulgation publique ; nous serons ravis de te créditer dans l'advisory.

## Périmètre — ce qui compte le plus

Zones à fort impact (traitées en priorité) :

- **Contournement de signature / vérification** — installer ou mettre à jour un paquet
  qui aurait dû être rejeté (signature Ed25519 invalide/absente alors que
  `require_signature` est actif).
- **Intégrité du paquet** — un `.bpkg`/SFX forgé qui passe la vérification mais écrit
  des octets différents de son manifest.
- **Path traversal / écriture arbitraire** — un manifest, un chemin de composant, ou une
  URL d'update qui s'échappe du dossier d'install ou écrit en dehors.
- **Abus du canal d'update** — attaques par downgrade, échange du paquet contre un
  paquet non signé/malveillant, ou récupération d'update en non-HTTPS.
- **Élévation de privilèges locale** au-delà du modèle par-utilisateur (`asInvoker`), ou
  exécution de code contrôlé par l'attaquant pendant l'install/update/désinstallation.

## Hors périmètre

- Les problèmes nécessitant une machine déjà compromise ou une *clé privée de signature*
  malveillante (protège `private.key` — voir [docs/SIGNING.md](docs/SIGNING.md)).
- La réputation SmartScreen/Gatekeeper (elle nécessite Authenticode / la notarisation
  Apple, orthogonale à la signature de paquet).
- L'ingénierie sociale poussant un utilisateur à lancer un exécutable sans rapport.

## Conseils de durcissement pour les intégrateurs

- Mets `[security].require_signature = true` et livre la vraie `public_key`.
- Garde la **clé privée hors-ligne** ; ne la commit jamais (elle est gitignorée).
- Héberge `update.json` et les paquets en **HTTPS** ; le nouveau paquet doit être signé
  par la **même clé** que celle à laquelle le build installé fait confiance.
- Valide tout ce que ton app lit depuis `installer-handoff.json` — traite-le comme une entrée.

## Versions supportées

Les correctifs de sécurité ciblent la dernière release (`main`). Les releases plus
anciennes sont corrigées au mieux.
