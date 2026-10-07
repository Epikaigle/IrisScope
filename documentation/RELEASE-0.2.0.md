# Iriscope 0.2.0 — installation personnelle

Cette version ajoute la progression et l’annulation des sauvegardes/restaurations,
l’historique des sauvegardes par dossier, un rappel facultatif après sept jours
et l’export d’une copie MP4 depuis la visionneuse vidéo. Les originaux AVI sont
conservés. Le numéro est visible dans les paramètres et avec `iriscope-app --version`.

## Installer ou mettre à jour

Fermer Iriscope et conserver une sauvegarde avant de remplacer le programme.
Installer le nouveau paquet ou remplacer l’ancienne application portable.
Les paramètres restent au même emplacement : `%APPDATA%/IrisScope/settings.json`
sous Windows, `~/Library/Application Support/IrisScope/settings.json` sous macOS
et `$XDG_CONFIG_HOME/IrisScope/settings.json` (ou `~/.config/IrisScope/settings.json`)
sous Linux. Les captures restent dans le dossier choisi dans l’application.
L’installateur ne les inclut pas et ne les supprime pas à la désinstallation.
Les nouveaux paramètres ont des valeurs par défaut compatibles avec les anciens
fichiers de configuration.

La signature officielle n’est pas requise pour l’usage personnel retenu.
Les paquets préparés sans option de signature ne demandent aucun certificat payant.

## Export MP4

Ouvrir une vidéo AVI dans la bibliothèque, choisir **Exporter MP4…**, puis son
emplacement. L’export est annulable et ne remplace jamais un fichier existant.
Il utilise FFmpeg avec l’encodeur H.264 `libx264`, installé sur le système ou
fourni à côté du programme dans un paquet qui l’embarque. Une installation
sans FFmpeg conserve toutes les fonctions de capture et de lecture AVI.

## Validation avant utilisation avec le DE400

Suivre [le protocole matériel](VALIDATION-MATERIELLE.md) sur l’ordinateur cible :
30 minutes de direct et d’enregistrement, déconnexion/reconnexion, changement
d’échelle entre écrans et installation/mise à jour. Les résultats cloud ne
remplacent pas ces essais. Le bouton physique sera traité sur l’appareil réel.
