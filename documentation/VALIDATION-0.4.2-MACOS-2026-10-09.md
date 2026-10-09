# Validation de la 0.4.2 sur macOS Intel — 9 octobre 2026

macOS 15.7.7 Intel, DE400 connecté. Les preuves locales restent dans
`target/updater-followup-20261009/`, hors Git.

## Correction et contrôles

- Le redémarrage Linux portable utilise le nouveau dossier de l’application.
  Un test lance un véritable processus avant et après suppression de la
  sauvegarde de l’ancienne version et vérifie son dossier de travail.
- Les neuf tests de l’updater passent en release et Clippy strict passe sur
  tous ses targets. Le formatage Rust et les différences Git passent.
- Dix-huit tests Python passent ; le test de construction Debian réelle est
  ignoré sur Mac. Les notes de la version courante sont vérifiées dans le paquet.
- Soixante-quatre rendus des paramètres correspondent aux références approuvées :
  quatre scènes, deux tailles, deux thèmes et quatre échelles. La version affichée
  dans les fixtures reste constante pour éviter de recalculer les références à
  chaque release ; l’application ordinaire affiche bien la version installée.
- Les contrôles du site passent. Aucun nouveau workflow GitHub n’est ajouté.

Une seule passe de compilation optimisée regroupe l’application, l’updater et
les fixtures (7 min 03 s). Les tests ciblés de l’updater prennent quelques secondes.
Les composants natifs USB existants sont réutilisés sans compilation. Le lecteur
USB et le service signé sont identiques à la 0.4.1 ; le lanceur reçoit la nouvelle
signature du bundle contenant les métadonnées 0.4.2.

## Installation réelle sur le Mac

La 0.4.1 installée est remplacée par la 0.4.2 à travers l’installateur de
production et un manifeste local signé avec la véritable clé de mise à jour.
Le bootstrap est lancé depuis l’ancien dossier `Contents/MacOS`, pour couvrir
le remplacement d’un dossier de travail situé dans l’application.

L’application redémarre, l’installateur se termine et son cache dans le dossier
temporaire macOS est nettoyé. Les 53 fichiers installés, leurs permissions et leur
signature correspondent au ZIP et au DMG vérifiés. Le JSON des réglages et les
quatre médias existants ont les mêmes empreintes avant et après.

L’observateur initial cherchait le cache dans `/private/tmp` au lieu du `TMPDIR`
macOS ; la vérification est complétée dans le bon dossier, sans refaire
l’installation. Un lancement du test pendant la fermeture de l’ancienne instance
affiche le message « déjà ouverte » ; l’essai est relancé après sa fermeture complète.

Le nouveau paquet USB est approuvé une fois avant l’essai. Les ouvertures suivantes
utilisent le compte normal UID 501, sans authentification. La signature ad hoc et
la signature Ed25519 sont gratuites ; aucune notarisation Apple n’est effectuée.

## Essai DE400 après installation

Le vrai flux USB alimente quatre PNG décodables de 1280 × 1024 et une vidéo AVI
de 32 images à 8 fps. Les yeux gauche/droit, les dossiers homonymes, la capture
sans dossier et la sauvegarde/restauration de cinq médias sont vérifiés.
Les neuf réglages caméra sont présents et le récepteur USB du bouton est actif.

Les déclenchements de ce complément sont logiciels, via le même gestionnaire
que le bouton. Aucun nouvel appui physique n’est demandé ; le lecteur et le
service USB conservent exactement les binaires précédemment testés.
L’export MP4 reste non testé sans le FFmpeg optionnel.

L’essai de l’installation intégrée Debian via Polkit/dpkg reste à faire sur
Linux, lors d’un essai séparé sur ce système. Windows et Apple Silicon restent
hors release.
