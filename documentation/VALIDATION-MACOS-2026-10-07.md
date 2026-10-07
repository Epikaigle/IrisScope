# Validation Mac du 7 octobre 2026

**Le bouton physique du DE400 fonctionne sur le Mac testé avec le chemin USB
ajouté au dépôt.** Le paquet release signé localement, installé depuis son DMG,
est validé avec quatre appuis réels : deux photos de 1280 × 1024 et une vidéo
AVI de 40 images lisibles, démarrée puis arrêtée par le bouton.
La sauvegarde et la restauration des trois médias passent également.

Le backend AVFoundation ordinaire reste inchangé pour l'accès au bouton :
le pilote Apple utilise déjà son interface de contrôle USB et l'ouverture
ordinaire renvoie `0xe00002c5`. Son premier essai physique n'avait produit
aucune photo. Le nouveau chemin reçoit directement la vidéo et le bouton USB
dans un composant lancé par le paquet natif avec authentification administrateur ;
l'interface graphique et les captures restent sous le compte utilisateur.

## Matériel et versions

- MacBookPro12,1, Intel Core i5-5257U, 8 Gio de RAM, macOS 15.7.7 (24G720).
- Digital Microscope / Infoxelle, USB `21cd:603b`, série `VTU603EB`, révision 3.27.
- Identifiant AVFoundation initial : `0x1410000021cd603b` ; après rebranchement : `0x1420000021cd603b`. La nouvelle réception USB recherche le modèle `21cd:603b`, sans imposer le numéro de série ni le port USB.
- Dépôt récupéré à `dd9c288846e04c71b247cbfba8b764a111ff5522`, version 0.4.0.
- Premier essai avec le paquet Intel officiel du workflow `37529803272`, commit
  `d1233ffdeb8be19cf76fd03c184a907c15043a02`. L'empreinte de l'artefact téléchargé
  correspond à celle fournie par GitHub. Le backend Mac est identique dans les
  deux commits.
- Correctifs locaux sur la branche `codex/macos-hardware-validation`.

## Défauts reproduits et correctifs

1. La caméra sélectionnait bien le mode 1280 × 1024, mais les buffers de sortie
   AVFoundation étaient réduits à 640 × 480. Une sonde indépendante reproduit la
   réduction et retrouve 1280 × 1024 en spécifiant les dimensions de sortie.
   Le backend fixe maintenant explicitement largeur et hauteur pour les formats
   décodés. Le chemin MJPEG natif reste inchangé.
2. `--validate-hardware` supposait que chaque photo était un JPEG, alors que le
   backend Mac enregistre les buffers décodés en PNG. Les quatre PNG du premier
   essai étaient valides ; c'était leur contrôle qui échouait. Le scénario
   utilise maintenant le décodeur commun JPEG/PNG.
3. `--diagnose` interrompait son attente au premier délai de 1,5 seconde, même
   pendant le démarrage du service caméra. Il attend maintenant dans sa limite
   globale de cinq secondes.
4. Le rapport matériel conserve désormais l'état USB du bouton, afin qu'un
   succès des captures logicielles ne masque pas le refus du bouton physique.
5. La première version du test de changements de qualité demandait 15 fps
   arrondis, alors que le pilote annonce exactement 15,000015 fps à 800 × 600.
   Le backend acceptait cette valeur proche mais hors plage, puis AVFoundation
   levait une exception native. Les demandes hors plage sont maintenant
   refusées avec `InvalidConfiguration`. Le test reprend les cadences exactes
   annoncées par la caméra, comme l'interface, et vérifie aussi ce refus.
6. Un passage du scénario complet s'arrêtait sur une lecture concurrente de la
   bibliothèque pendant la publication des photos (`NotFound`). Les trois
   photos étaient enregistrées correctement. Le scénario réessaie maintenant
   les lectures transitoires (`NotFound`, `WouldBlock`, `Interrupted`) au prochain
   tick, dans sa limite globale de 45 secondes ; les autres erreurs restent
   bloquantes.

Les premiers journaux et sondes se trouvaient sous `target/macos-validation/` ;
un nettoyage utilisateur a supprimé ce dossier pendant la reconstruction.
Les résultats historiques ci-dessous décrivent les essais observés avant ce
nettoyage. Les nouvelles compilations, journaux et captures fictives sont
conservés sous `target/macos-button-research/`, dans un cache local auquel
`target` renvoie désormais. Les scénarios ne modifient pas les captures ni les
paramètres personnels.

## Première validation AVFoundation

La compilation locale utilise Rust 1.89.0, le lockfile du dépôt et le profil
`dev`, avec le décodeur caméra optimisé comme prévu par le workspace.

- `--diagnose` : trois images NV12 de 1280 × 1024 reçues et décodées ; refus USB
  du bouton confirmé séparément.
- `--validate-hardware target/macos-validation/final-confirmed` : **PASS**.
  Quatre photos PNG de 1280 × 1024, une vidéo AVI de 39 images toutes lisibles à
  une cadence déclarée de 8 fps, deux dossiers homonymes distincts et une capture
  anonyme. Les deux côtés d'œil sont correctement associés.
- Les cinq médias, leurs noms et leurs associations sont préservés à l'identique
  par la sauvegarde et la restauration. L'AVI original reste inchangé.
- Export MP4 : **NON TESTÉ**, FFmpeg absent du poste.
- Tests Python du paquet et de la passerelle : neuf réussis, un test Debian
  ignoré car les outils Debian ne sont pas disponibles sur Mac.
- Tests Rust exécutés nativement sur Mac : **204 réussis**, aucun échec
  (application : 67, backend Mac : 15, cœur : 105, traitement d'image : 17).
  Les 15 tests du backend ont été relancés après le correctif de cadence.
- Test matériel supplémentaire : **PASS**, trois images valides dans chacun
  des modes 1280 × 1024 à 8 fps, 800 × 600 à 15,000015 fps,
  640 × 480 à 30,000030 fps, puis retour à 1280 × 1024.
  La demande arrondie de 15 fps hors plage est refusée sans exception native.
  Le test est ignoré par défaut et doit être lancé explicitement avec le DE400
  connecté ; résultat dans `target/macos-validation/camera-mode-final.txt`.
- Clippy sur toutes les cibles de l'application et du backend Mac, avec
  `-D warnings` : **PASS**, aucun avertissement. `cargo fmt --all -- --check`
  et `git diff --check` passent également.

Le rapport détaillé est dans
`target/macos-validation/final-confirmed/validation-report.txt`. Le paquet local
`target/macos-validation/final/IrisScope.app` contient la version corrigée ;
il s'agit d'une compilation de développement, non d'un installateur de release.

## Rejouer les essais

Depuis la racine du dépôt, avec le DE400 connecté et sans autre application
utilisant la caméra :

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build --locked -p iriscope-app -j 2
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test --locked -p iriscope-camera-macos --test de400_hardware -j 2 -- --ignored --nocapture
SLINT_BACKEND=winit-software target/debug/iriscope-app --validate-hardware target/macos-validation/nouvel-essai
```

Le dernier chemin doit être absent ou vide. `--validate-hardware` déclenche les
captures par logiciel et ne valide pas la réception physique du bouton. Pour
un nouvel essai avec des appuis réels, utiliser `--validate-button` et un autre
dossier vide.

## Bouton : accès trouvé et intégré

Les étapes ont été vérifiées sur cet appareil, pas seulement déduites du SDK :

1. Les trois appuis pendant la lecture de `VS_STILL_TRIGGER_CONTROL` ne changent
   pas son état, toujours zéro. Les notifications CoreMediaIO surveillées ne
   signalent pas non plus le bouton. Ce contrôle ne constitue pas un état
   exploitable du bouton physique.
2. `USBInterfaceOpenSeize` est également refusé, y compris dans la sonde exécutée
   avec les droits administrateur.
3. `USBDeviceReEnumerate(kUSBReEnumerateCaptureDeviceMask)`, suivi de la
   réouverture du périphérique, permet réellement d'ouvrir l'interface 0 et
   l'endpoint d'interruption `0x81`. Les trois appuis donnent chacun les quatre
   octets `02 01 00 01`, puis `02 01 00 00` au relâchement. Le périphérique est
   rendu à macOS avec `kUSBReEnumerateReleaseDeviceMask`.
4. Cette réservation retire également le pilote vidéo Apple. Le restituer
   alors que le canal du bouton reste ouvert annule la lecture du bouton.
   Un simple lecteur de bouton à côté d'AVFoundation ne suffit donc pas ici.
5. Une réception UVC directe prend en charge les deux canaux. Le DE400 place
   ses descripteurs vidéo après le descripteur d'endpoint ; libusb les rattache
   à l'endpoint, alors que libuvc ne cherchait que dans l'interface. Le patch
   local lit aussi cet emplacement et contrôle les longueurs des descripteurs.
6. Le composant transmet des images YUYV complètes et les événements physiques
   à IrisScope par une socket locale réservée au compte utilisateur. Le backend
   Rust contrôle le protocole, la longueur exacte des images et des rapports,
   et utilise le même décodeur de bouton et la même file d'événements que le
   récepteur existant. Les images incomplètes sont rejetées.

L'inspection du framework de [FireflyPro Mac 2.4](https://fireflyglobal.com/downloads-os-x-wired/)
retrouve un accès IOKit ordinaire au même endpoint, sans solution supplémentaire
identifiée dans les routines de réception examinées. Le framework propriétaire
n'est ni intégré ni redistribué. La règle d'accès administrateur est documentée
dans le [SDK Apple IOUSBLib](https://github.com/apple-oss-distributions/IOUSBFamily/blob/main/IOUSBFamily/Headers/IOUSBLib.h).

Le composant utilise libusb 1.0.30 et libuvc au commit
`d07de4fa23905ee8cac06f5d24b2a03d42a3b363`, avec 64 transferts en file.
Les bibliothèques et leurs sources restent sous `target/macos-button-research/`.
Le script de compilation épingle l'archive libusb par SHA-256 et conserve le
patch libuvc dans `scripts/macos/libuvc-de400-descriptors.patch`.

### Résultat du scénario physique

`target/macos-button-research/integrated-button-validation/validation-report.txt`
indique **Physical button: true** et **PASS**. Aucune capture de ce scénario
n'est déclenchée par simulation logicielle : l'opérateur effectue les quatre
appuis. Le journal du composant relève quatre appuis et quatre relâchements,
235 images complètes transmises et aucune image incomplète sur ce passage.

- Photo gauche : 1280 × 1024, dossier D-000001.
- Photo droite : 1280 × 1024, dossier D-000002, mêmes noms mais identité distincte.
- Vidéo droite : 42 images toutes lisibles, cadence déclarée de 8 fps, dossier
  D-000001. L'AVI original est conservé et les trois médias restaurés ont les
  mêmes octets et associations.
- Après fermeture du composant, le diagnostic AVFoundation ordinaire reçoit
  et décode de nouveau trois images NV12 de 1280 × 1024. Le retour de la vidéo
  au pilote Apple est donc vérifié, et son ancien refus USB du bouton reste
  explicitement signalé.
- Après ajout du chemin USB, les tests du backend Mac et du cœur sont relancés :
  **18 + 105 réussis**, aucun échec. Les trois nouveaux tests vérifient les
  paquets fragmentés avec délais, le refus des images incomplètes/paquets trop
  grands et le refus d'un relâchement tronqué. Clippy sur toutes les cibles de
  l'application et du backend Mac passe avec `-D warnings`. La reconstruction
  épinglée du composant C passe aussi avec `-Wall -Wextra -Werror`.

### Paquet Mac natif et réglages

Le paquet inclut désormais trois exécutables : `IrisScope`, lanceur natif,
`IrisScopeGui`, application Rust, et `de400-usb-helper`, composant vidéo/bouton/
réglages. Un double-clic lance l'authentification administrateur macOS puis
l'interface sous le compte utilisateur. Aucun mot de passe n'est enregistré.
L'utilisateur n'a besoin ni de Python, ni de Rust, ni de bibliothèque système
supplémentaire. La fermeture de l'application arrête le composant USB et rend
l'appareil au pilote Apple. Une seconde instance est bloquée pour éviter que
les deux applications essaient d'utiliser la même caméra.

La session USB ne possède plus de limite horaire. Le lanceur démarre le
composant en arrière-plan après l'autorisation ; la fin de sa propre vie est
contrôlée par PID et date de démarrage, plutôt que par un chronomètre. Il peut
rouvrir la caméra après fermeture d'une connexion, sans nouvelle autorisation.
Un contexte libuvc neuf par connexion corrige un arrêt du thread d'événements
reproduit lors de la deuxième ouverture. Le composant est lié statiquement à
libusb/libuvc et n'installe aucun pilote ou service permanent.

Les huit contrôles effectivement annoncés par ce DE400 sont exposés :

| Réglage | Plage | Valeur par défaut |
| --- | --- | --- |
| Luminosité | 0–255 | 127 |
| Contraste | 0–31 | 15 |
| Saturation | 0–31 | 15 |
| Teinte | −180–180 | 0 |
| Netteté | 0–15 | 13 |
| Gamma | 0–127 | 64 |
| Balance des blancs | 0–255, unité non annoncée | 128 |
| Anti-scintillement | Désactivé / 50 Hz / 60 Hz | 60 Hz |

Le test matériel `de400_usb_hardware` passe pendant la vidéo : lecture de
chaque réglage, modification et relecture, restauration, refus des valeurs
hors plage, réinitialisation aux valeurs par défaut et restauration des
valeurs personnelles. Deux cycles ouverture/vidéo/fermeture réussissent avec
le même composant autorisé. Le journal est dans
`target/macos-button-research/hardware-controls-final.txt`. Aucun contrôle
non annoncé (exposition automatique, gain, etc.) n'est inventé dans l'interface.

La limite arbitraire d'une heure des enregistrements est également supprimée.
Le test du calcul de durée simule deux jours de frames continues. Le test C
vérifie la poursuite de la session au-delà d'une heure et d'une journée et son
arrêt à la fermeture de la connexion. Ces tests ne constituent pas un essai
physique de deux jours. Le conteneur AVI classique conserve sa limite de taille
(environ 4 Gio) et un flux caméra interrompu trop longtemps arrête la vidéo
avec un message, en préservant le fichier temporaire récupérable.

Les tests locaux passent : **211 tests Rust** (68 application exécutés avant
le nettoyage ; 21 backend Mac, 105 cœur et 17 traitement d'image relancés après),
plus **12 tests Python** et un test Debian ignoré faute d'outils sur ce Mac.
La compilation C/Objective-C avec
`-Wall -Wextra -Werror` et les tests de durée/fermeture de socket passent.
Les nouveaux journaux restent sous `target/macos-button-research/`.

### Validation du paquet release installé

La compilation Rust finale utilise le profil `release` optimisé. Le ZIP et le
DMG sont créés, signés ad hoc et vérifiés par `codesign --verify --deep --strict`.
Le DMG est vérifié puis monté en lecture seule ; son application est copiée
dans `~/Applications/IrisScope.app`. Les essais suivants utilisent cette
application installée, avec le DE400 réellement connecté :

- Le diagnostic décode trois images YUYV de 1280 × 1024 et annonce la réception
  USB du bouton ainsi que huit contrôles disponibles.
- Le scénario logiciel complet conserve quatre photos, une vidéo de 35 images
  lisibles, les associations aux yeux/dossiers et les cinq médias après
  sauvegarde/restauration à l'identique.
- **Le scénario physique passe** : quatre appuis et quatre relâchements reçus,
  deux photos PNG de 1280 × 1024, démarrage puis arrêt d'une vidéo AVI de
  40 images lisibles à 8 fps. Les trois médias sont restaurés avec exactement
  les mêmes octets et associations ; l'AVI original reste inchangé.
- L'interface annonce les huit réglages : luminosité, contraste, saturation,
  teinte, netteté, gamma, balance des blancs et anti-scintillement. Leur lecture,
  modification, relecture, réinitialisation et restauration sont validées
  pendant la vidéo par le test matériel du composant.
- Le journal du scénario physique reçoit 457 images complètes et aucune
  image incomplète. La fermeture arrête le composant USB ; le diagnostic
  AVFoundation ordinaire reçoit ensuite trois images NV12 de 1280 × 1024.

Les rapports sont `target/macos-button-research/release-integration-20261007-181944/validation-report.txt`
et `target/macos-button-research/release-button-20261007-181944/validation-report.txt`.
Le journal USB est `target/macos-button-research/release-helper.txt` et le
contrôle du retour au pilote Apple est `target/macos-button-research/av-restored-final.txt`.

### Reconstruire et lancer

Depuis la racine du dépôt :

```sh
python3 scripts/macos/build-usb-experiment.py
cargo build --release --locked -p iriscope-app
python3 scripts/package-release.py
python3 scripts/package-installer.py
```

Le profil Rust est `release` optimisé ; le lanceur et le composant sont compilés
avec `-O2` et libuvc avec le profil CMake `Release`. Les scripts créent une
archive ZIP et un DMG avec leurs empreintes dans `dist/`. CMake et les outils
Apple sont nécessaires pour construire, sans installation de service USB.
Les sources et licences de libusb/libuvc, le patch et les objets permettant
la réédition des liens sont inclus dans `Contents/Resources/USB-sources`.

Les nouveaux paquets sont signés localement **ad hoc**, gratuitement. Les
composants internes sont signés avant le bundle, puis `codesign --verify
--deep --strict` vérifie le résultat. Cette signature ne constitue ni une
identité Developer ID ni une notarisation Apple ; elle ne dispense pas de
l'authentification USB. Aucun abonnement payant n'est utilisé pour ce poste.

`--avfoundation` sélectionne l'ancien chemin pour les diagnostics comparatifs.
`IRISCOPE_DE400_SOCKET` est une entrée réservée aux tests qui disposent déjà
d'un composant autorisé ; elle n'accorde aucun privilège. Le paquet ordinaire
n'a besoin d'aucune variable d'environnement.

## Limites de la validation

Les quatre appuis physiques, les photos, l'AVI et la sauvegarde/restauration
ont été validés sur le paquet release final installé depuis le DMG. Les
contrôles, leur réinitialisation et la réouverture sont également validés
sur le nouveau protocole USB.

Le mode USB direct reste 1280 × 1024 YUYV à 8 fps. Windows et Apple Silicon ne
sont pas testés physiquement sur ce poste. Les sessions d'une journée,
la déconnexion physique, la veille/reprise, le redémarrage du Mac et tous les
parcours manuels de la visionneuse ne sont pas couverts par les essais actuels.
L'export MP4 reste non testé sans FFmpeg.
