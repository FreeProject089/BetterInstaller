# BetterInstaller — Fonctionnalités

> Ce que fait **BetterInstaller**, fonctionnalité par fonctionnalité. Détail technique
> dans **Technical_Analysis_FR.md** ; revue de menaces dans **Security_Audit_FR.md**.

## Pour les auteurs d'app
- **Une config → un installeur.** Décrivez votre app dans un seul `installer.toml`
  (nom, version, éditeur, branding, composants, raccourcis, prérequis, sources de mise à
  jour) et produisez un unique `Setup.exe` auto-extractible.
- **Paquets `.bpkg`** — votre payload est empaqueté dans un seul fichier signé,
  compressé zstd, auto-descriptif, avec un manifeste SHA-256 par fichier.
- **Signature Ed25519** — `bpkg keygen` / `sign` ; l'installeur vérifie contre une clé
  publique épinglée dans la config, et peut être réglé pour **abandonner si la signature
  est absente/invalide**.
- **Binaire minuscule** — profil release réglé pour un installeur <5 Mo, sans runtime
  WebView.
- **CLI** — `bpkg keygen · pack · sign · stamp · verify` pour le packaging scripté/CI.
- **Multiplateforme** — backends Windows, Linux, macOS derrière un seul moteur.

## Pour les utilisateurs finaux (l'installeur GUI)
- **UI native Slint** — pas de navigateur/WebView, rapide et léger.
- **Aucun admin requis (Windows)** — install par-utilisateur dans
  `%LOCALAPPDATA%\Programs`, seul `HKCU` est touché (pas d'invite UAC, aucun changement
  système).
- **Licence & composants** — consulter les documents de licence (lus directement depuis
  le paquet avant extraction) et choisir les composants optionnels.
- **Install vérifiée** — le hash de chaque fichier est vérifié contre le manifeste
  **avant** l'écriture ; la signature du paquet est vérifiée quand une clé publique est
  configurée.
- **Raccourcis & intégration** — raccourcis menu Démarrer/bureau, enregistrement du
  gestionnaire de protocole, entrée Ajout/Suppression de programmes, entrée PATH
  optionnelle.
- **Handoff au premier lancement** — les choix d'install sont transmis à l'app pour
  qu'elle démarre déjà configurée (pas de second assistant).
- **Progression** — progression throttlée, fichier par fichier, pendant l'extraction.

## Mises à jour & maintenance
- **Mise à jour auto** — vérifie un ou plusieurs miroirs de manifeste et propose la
  version la plus récente (un miroir mort ne bloque jamais les autres).
- **Deltas binaires** — télécharge un petit patch depuis votre version courante quand
  c'est proposé, au lieu du paquet complet.
- **Sûr au rollback** — une mise à jour snapshot d'abord le dossier d'install et **le
  restaure en cas d'échec**, pour qu'une mise à jour cassée ne laisse pas une app à
  moitié installée.
- **Réparer** — re-vérifie et restaure la version courante.
- **Désinstallation propre** — retire fichiers, raccourcis, gestionnaire de protocole et
  entrées de registre.
