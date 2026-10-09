# IrisScope 0.4.1

Cette version conserve les corrections d’interface et ajoute les mises à jour
depuis **Paramètres** pour macOS Intel (macOS 15+) et Linux x86_64.

- Vérification automatique de la dernière release stable au démarrage.
- Téléchargement depuis l’application, avec vérification de signature Ed25519
  et de l’empreinte SHA-256 avant toute installation.
- **Installer et redémarrer** attend la fin des captures, des notes et des
  sauvegardes. Les données restent hors des fichiers remplacés.
- macOS et Linux portable conservent l’ancienne application jusqu’à confirmation
  du démarrage ; une erreur de remplacement ou de lancement restaure l’ancienne.
- Linux Debian utilise le gestionnaire de paquets avec authentification système.
- Une nouvelle version Mac peut demander une authentification USB ; les
  ouvertures suivantes la réutilisent. Aucun abonnement Apple n’est nécessaire.

Windows et Apple Silicon ne sont pas publiés dans cette release. Voir
[Mises à jour](https://github.com/Epikaigle/iriscope-app/blob/main/documentation/MISES-A-JOUR.md)
pour l’installation et la publication.
