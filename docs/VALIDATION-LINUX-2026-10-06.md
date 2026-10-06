# Validation matérielle Linux du 6 octobre 2026

Essais réalisés sur le PC local avec le DE400 Infoxelle / Firefly branché en USB,
VID/PID `21cd:603b`, révision 3.27, UVC 1.00, pilote `uvcvideo`, noyau
`6.8.0-146-generic`. Application 0.4.0 compilée en release ; interface Slint
exécutée sous Xvfb avec les contrôleurs, workers et fichiers de production.
Les noms de dossiers utilisés sont fictifs. Les réglages et captures personnels
ne sont pas utilisés par les scénarios.

## Signal du bouton

Trois pressions coordonnées avec l'utilisateur ont chacune produit le paquet
`02 01 00 01`, puis `02 01 00 00` au relâchement, sur l'endpoint d'interruption
`0x81`. Aucun périphérique de touches ni marqueur STILL_IMAGE dans les
métadonnées vidéo n'a été observé. Le descripteur annonce `bTriggerUsage = 0`.
La [passerelle Linux](../helpers/linux/README.md) est installée, active et
configurée pour démarrer avec le PC.

Un premier essai complet a révélé une perte des signaux pendant le direct :
`usbmon` texte retourne un seul événement par lecture. La passerelle vide
désormais sa file par lots bornés avant de dormir, afin que le trafic vidéo
n'évince pas les événements du bouton. Le test physique ci-dessous a réussi
après cette correction. Le service consomme environ 1–2 % de CPU pendant le
direct lors de cet essai.

## Résultats obtenus

| Parcours | Résultat |
| --- | --- |
| Identification, modes et dix contrôles natifs lisibles | Réussi |
| Acquisition et décodage de trois frames MJPEG 1280 × 1024 | Réussi |
| Élimination des frames anciennes après une pause du consommateur | Réussi |
| Appui physique maintenu environ 2,5 secondes | Une seule photo gauche, 1280 × 1024 |
| Deuxième appui physique | Une photo droite, 1280 × 1024 |
| Troisième puis quatrième appui | Démarrage puis arrêt d'une vidéo AVI : 78 frames décodables, cadence du conteneur 8 FPS |
| Deux dossiers homonymes | Identités distinctes D-000001 et D-000002 ; chaque capture conserve le dossier et l'œil attendus |
| Noms des fichiers | Prénom, nom, œil, date et heure présents |
| Deux photos automatiques dans la même seconde | Noms distincts, dont un suffixe `_2`, sans écrasement |
| Capture anonyme automatique | Préfixe `Iris`, œil conservé, aucun dossier attribué |
| Arrêt automatique après passage du mode Vidéo au mode Photo | Vidéo finalisée ; aucune photo accidentelle |
| Export MP4 des deux vidéos d'essai | Réussi ; AVI original inchangé |
| Sauvegarde puis restauration des captures automatiques et physiques | Médias identiques octet par octet, dossiers et yeux conservés |

La résolution maximale annonce 8 FPS ; les horodatages des sondes mesurent
environ 6,25 images distinctes par seconde sur cet exemplaire. Le conteneur vidéo
maintient la durée grâce à la répétition d'images lorsque nécessaire. Aucun mode
1280 × 1024 à 30 FPS n'est exposé.

## Vérifications logicielles

- `cargo fmt --all --check` et Clippy strict sur tous les targets du workspace natif : réussis.
- `SLINT_BACKEND=winit-software xvfb-run -a cargo test --release --workspace --locked -j2` : 225 tests réussis ; les deux tests réservés au DE400 ont ensuite été exécutés séparément et réussissent.
- Tests Python des paquets et de la passerelle : réussis. Les archives et le paquet Debian contiennent les fichiers nécessaires à l'installation manuelle de la passerelle.
- Le test d'export d'une frame JPEG corrompue réussit avec FFmpeg 6.1.1 : aucun MP4 partiel publié.

Le backend logiciel est nécessaire pour les tests de rendu qui prennent un
instantané d'une fenêtre cachée sous Xvfb ; le backend OpenGL par défaut refusait
cet instantané car son renderer était suspendu. Les assertions restent actives.

## Traces locales et limites

Les rapports, médias et sauvegardes d'essai sont exclus de Git :

```text
target/hardware-validation-2026-10-06/
├── automatic/validation-report.txt
├── physical-button-2/validation-report.txt
├── physical-button-2/button-events.json
├── diagnose-final.txt
├── clippy.log
└── native-tests-software.log
```

Le stockage conserve les médias dans le dossier de captures et leur classement
par numéro de dossier dans `.iriscope-index.json`. Sauvegarder tout le dossier,
y compris les fichiers cachés, conserve cette organisation.

La déconnexion/reconnexion physique, le redémarrage du PC, un direct de
30 minutes et les essais matériels Windows/macOS n'ont pas été réalisés dans
cette session. Le [protocole complet](VALIDATION-MATERIELLE.md) reste applicable
avant une diffusion sur ces plateformes.
