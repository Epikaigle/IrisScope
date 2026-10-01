# IrisScope

IrisScope est une application desktop native et multiplateforme conçue pour utiliser simplement un **iridoscope Firefly DE400 / Infoxelle Digital Microscope** sous **Linux, Windows et macOS**.

L'objectif est de remplacer l'ancien logiciel FireflyPro par une application moderne, rapide et très simple à utiliser : on branche le DE400, l'image apparaît immédiatement, on renseigne la personne examinée et l'œil, puis on capture les photos ou vidéos. IrisScope s'occupe automatiquement du nommage, du classement et de la bibliothèque.

> IrisScope est un logiciel de capture, de visualisation et d'organisation d'images. L'utilisateur peut ajouter ses propres images de référence d'iridologie ; aucune planche n'est fournie avec l'application. Ces images sont uniquement des références visuelles et ne constituent pas un outil de diagnostic médical.

## Expérience utilisateur visée

L'écran **Caméra** est l'écran principal et doit suffire pour la grande majorité de l'utilisation.

Au lancement, IrisScope doit :

1. détecter automatiquement le Firefly DE400 ;
2. ouvrir le meilleur flux vidéo disponible sans demander de choisir une caméra ;
3. afficher immédiatement le live ;
4. permettre de retrouver ou de créer son dossier à partir du prénom et du nom ;
5. permettre de choisir **œil gauche** ou **œil droit** ;
6. permettre de prendre une photo ou de démarrer / arrêter une vidéo depuis l'interface ;
7. enregistrer et classer automatiquement le fichier ;
8. rendre immédiatement la capture accessible dans la bibliothèque.

Aucune étape technique de type « ouvrir la caméra », « choisir /dev/video0 » ou « sélectionner un codec » ne doit être nécessaire en usage normal.

## Fonctions disponibles et en cours

### Caméra et live

- connexion automatique au Firefly DE400 ;
- affichage live dès l'ouverture de l'application ;
- mode **Photo** ou **Vidéo** clairement sélectionnable ;
- gros bouton logiciel de capture ;
- le bouton physique du DE400 n'est pas encore relié à la capture dans les backends ;
- indicateur **REC** et durée d'enregistrement ;
- dernière capture visible immédiatement ;
- détection de déconnexion / reconnexion de la caméra ;
- conservation d'une latence minimale grâce à une stratégie « dernière frame disponible ».

### Photos

- capture depuis le flux actif, à sa résolution effective. Le réglage de qualité vidéo peut donc aussi modifier la résolution des photos ;
- conservation du JPEG natif du DE400 lorsque la caméra fournit du MJPEG, sans réencodage inutile ;
- prise en charge des formats décodés lorsque le système d'exploitation ne fournit pas directement le MJPEG ;
- nommage automatique ;
- aperçu intégré ;
- accès depuis la bibliothèque.

Exemple de nom :

```text
Jean_Dupont_Droit_2026-09-20_18-42-16.jpg
```

### Vidéos

- enregistrement MJPEG AVI depuis le flux du DE400 ;
- conservation directe des frames quand le backend fournit du MJPEG natif ;
- encodage JPEG lorsque le backend fournit une image décodée plutôt que du MJPEG natif ;
- démarrage / arrêt depuis l'interface ;
- finalisation propre du fichier en cas de déconnexion de la caméra ;
- classement dans la même bibliothèque que les photos ;
- lecteur vidéo intégré avec lecture / pause, position, durée et navigation dans la timeline ;
- cadence de lecture compensée par le temps de décodage pour rester proche du FPS enregistré ;
- durée préservée malgré les images perdues, en maintenant la dernière image jusqu'au prochain horodatage caméra ;
- affichage du temps d'enregistrement.

Un enregistrement est limité à une heure. Une interruption trop longue de la caméra arrête l'enregistrement avec un message et conserve le fichier temporaire récupérable.

### Session et nommage

La saisie reste volontairement minimale :

- prénom et nom pour une capture associée à un dossier ;
- œil gauche / droit.

Une capture anonyme reste possible sans prénom ni nom. Dès qu'un des deux champs est renseigné, les deux doivent l'être pour choisir un dossier. Aucun autre renseignement personnel n'est demandé.

Pendant la saisie, IrisScope propose les dossiers dont les noms correspondent. L'utilisateur choisit **Utiliser** pour retrouver une personne déjà enregistrée ou **Créer une autre personne** pour un homonyme. Deux dossiers peuvent donc porter exactement les mêmes prénom et nom : chacun reçoit un numéro stable, par exemple `D-000042`. La date de dernière capture apparaît dans les résultats lorsqu'elle est disponible. L'application ne choisit et ne fusionne jamais deux homonymes automatiquement. Le numéro du dossier sélectionné reste visible pendant la session et les nouvelles captures lui sont associées ; il suffit de choisir une fois le dossier pour une série de prises de vue.

Pour revoir un patient, saisir son prénom et son nom puis sélectionner le bon numéro, ou saisir directement son numéro dans le champ « Retrouver par n° de dossier ». Cette recherche directe reste disponible même s'il existe de nombreux homonymes. En cas de doute, consulter ses captures dans la bibliothèque avant d'enregistrer de nouvelles images. Le numéro de dossier est l'identifiant fiable : les noms servent à la recherche. Les captures anciennes qui n'ont pas de numéro restent sans dossier jusqu'à une attribution explicite.

Le dossier et l'œil restent actifs jusqu'à ce que l'utilisateur les change afin de pouvoir réaliser plusieurs captures successives rapidement. Utiliser **Terminer** avant de commencer avec une autre personne.

Les fichiers sont automatiquement nommés avec, lorsqu'ils sont renseignés :

- le nom de la personne ;
- le côté de l'œil ;
- la date ;
- l'heure.

### Bibliothèque

IrisScope doit permettre de retrouver les captures sans devoir parcourir les dossiers du système.

La bibliothèque propose actuellement :

- photos et vidéos réunies au même endroit ;
- miniatures pour les photos et pour les vidéos AVI à partir de leur première image ;
- affichage au choix en grille ou en liste, avec sélection puis ouverture dans la visionneuse ;
- filtres Tous / Photos / Vidéos / Dossier sélectionné ;
- ouverture initiale sur toutes les captures, puis conservation du filtre et de la page entre les onglets ;
- masquage du nom dans le titre des captures appartenant à d'autres personnes ; les noms de fichiers et les miniatures restent visibles dans le dossier de stockage ;
- index local caché de métadonnées et de numéros de dossier, indépendant du modèle de nom de fichier ;
- visionneuse photo intégrée avec zoom / déplacement ;
- lecteur vidéo MJPEG AVI intégré avec timeline et recherche dans la vidéo ;
- ouverture des autres vidéos détectées (par exemple MP4 ou MKV) avec l'application associée du système ;
- accès rapide au dossier de stockage ;
- pagination de 100 captures, avec réutilisation du tri et des filtres entre les pages ; les fichiers visibles sont revérifiés avant affichage, et **Actualiser** force une relecture complète ;
- accès à la carte d'iridologie depuis la bibliothèque, dans une fenêtre refermable.

Le stockage reste local à l'ordinateur. Aucun compte ou cloud n'est requis. Les
fichiers, leurs noms, les miniatures affichées et l'index de métadonnées ne
sont pas chiffrés par IrisScope : les personnes ayant accès au dossier de
captures peuvent consulter ces informations. Le masquage des titres dans la
bibliothèque est une aide visuelle, pas une protection d'accès.

**Sauvegarde :** copier le dossier de captures dans son ensemble, y compris les fichiers cachés comme `.iriscope-index.json`. Les numéros de dossier et les associations entre patients et captures se trouvent dans cet index : sauvegarder uniquement les photos et vidéos ne suffit pas à conserver ces liens. Garder les copies sur un support dont l'accès est maîtrisé. Les captures interrompues peuvent laisser des fichiers cachés `.part`. La bibliothèque signale leur présence et propose **Récupérer les vidéos interrompues** : les images encore lisibles sont publiées sous un nom `Iris_Recuperee_…`, sans dossier patient attribué. Vérifier la vidéo puis la rattacher explicitement au bon dossier. Conserver les fichiers `.part` lors d'une restauration ; ne pas les renommer en `.avi` à la main. L'application ne chiffre ni les sauvegardes ni les supports amovibles.

Lors de la publication d'une capture, un journal caché conserve le dossier choisi avant l'écriture du fichier final. Si l'application s'interrompt entre l'enregistrement du média et la mise à jour de l'index, la reprise termine cette association automatiquement. Conserver aussi ces fichiers cachés dans les sauvegardes. Une capture modifiée ou remplacée depuis l'interruption n'est pas réattribuée automatiquement : le message d'erreur préserve les fichiers pour vérification.

Les associations enregistrées sont protégées par une empreinte du contenu et une version du fichier. Une copie identique restaurée depuis une sauvegarde conserve son dossier ; un contenu différent sous le même nom est désassocié. Au premier chargement d'un ancien index, les empreintes manquantes sont établies pour les fichiers encore compatibles avec leur taille et leur date enregistrées. Une modification antérieure à cette première empreinte ne peut pas être identifiée rétrospectivement. Si cette mise à jour ne peut pas être enregistrée, les anciennes associations dépourvues d'empreinte ne sont pas affichées comme vérifiées. Sur un système de fichiers qui ne fournit pas de version fiable, la vérification doit relire le contenu et peut ralentir le chargement de grandes vidéos dans la bibliothèque ; leur lecture est proposée dans l'application du PC.

Les écritures très rapprochées peuvent partager les mêmes dates sur disque. Les empreintes de fichiers récemment modifiés ne sont donc pas mises en cache. Le lecteur vidéo attend brièvement la stabilité du fichier en arrière-plan, vérifie son contenu, puis utilise des contrôles de métadonnées pendant la lecture.

### Réglages de l'image

Les réglages s'ouvrent dans une petite fenêtre en haut à gauche de l'aperçu caméra. L'image reste visible pendant le déplacement des curseurs, et les valeurs choisies sont conservées automatiquement pour les prochains lancements.

Lorsque la caméra / le système le permet, IrisScope agit directement sur les contrôles du DE400 plutôt que d'appliquer artificiellement un filtre après capture.

Réglages visés :

- luminosité ;
- contraste ;
- saturation ;
- teinte ;
- gamma ;
- netteté ;
- balance des blancs automatique / manuelle ;
- fréquence secteur 50 / 60 Hz ;
- exposition lorsque disponible.

Fonctions d'affichage supplémentaires :

- freeze du live ;
- zoom d'affichage ;
- curseur de zoom continu de 100 à 200 %, avec accès direct à 100, 150 et 200 % ;
- rotation ;
- miroir ;
- transformations non destructives pour l'original.

### Références d'iridologie

IrisScope propose un accès rapide à deux images choisies par l'utilisateur :

- carte / planche de l'iris ;
- planche de signes / symboles ;
- zoom et déplacement dans les images.

Aucune image de référence n'est livrée avec le logiciel : la recherche n'a pas trouvé de paire cartographie/symboles offrant à la fois une qualité adaptée et une autorisation explicite de redistribution. Chacun peut choisir dans **Réglages** des images qu'il est autorisé à utiliser. Ces éléments sont uniquement des aides visuelles de consultation.

### Réglages généraux

La page **Réglages** permet notamment de consulter ou modifier :

- état de connexion et diagnostic du DE400 ;
- dossier d'enregistrement ;
- modèle de nommage avec les jetons `{prenom}`, `{nom}` et `{oeil}` obligatoires dans le modèle ; les champs patient peuvent rester vides lors d'une capture, et `{date}` / `{heure}` sont facultatifs ;
- qualité du flux vidéo et thème clair, sombre ou selon le système ;
- chemins des deux images de référence d'iridologie.

Les images de référence sont limitées à 32 Mio sur disque et 16 mégapixels au décodage pour éviter une allocation excessive en mémoire.

Les contrôles image réellement exposés par le backend sont générés dynamiquement dans la fenêtre **Réglages image** de la caméra.

## Interface volontairement simple

L'application comporte trois espaces principaux :

1. **Caméra**
2. **Bibliothèque**
3. **Paramètres et diagnostic**

Le panneau patient est à gauche de l'image et peut être masqué pour agrandir l'aperçu. Les réglages d'image s'ouvrent au-dessus du direct et la carte d'iridologie s'ouvre depuis la bibliothèque.

### Raccourcis clavier

| Raccourci | Action |
| --- | --- |
| `Ctrl+1` | Afficher la caméra |
| `Ctrl+2` | Afficher la caméra et ouvrir ou fermer les réglages d'image |
| `Ctrl+3` | Ouvrir la bibliothèque |
| `Ctrl+4` | Ouvrir la carte d'iridologie depuis la bibliothèque |
| `Ctrl+5` | Ouvrir les paramètres et le diagnostic |
| `Ctrl+P` | Prendre une photo depuis l'onglet Caméra |
| `Ctrl+R` | Démarrer ou arrêter l'enregistrement vidéo depuis l'onglet Caméra |
| `Ctrl+F` | Figer ou reprendre l'aperçu caméra |
| `Échap` | Fermer la visionneuse, la carte ou les réglages d'image |

Les raccourcis de capture exigent un flux actif et un œil sélectionné. Les captures sans nom sont autorisées et reçoivent un nom de fichier commençant par `Iris`. Les raccourcis sont désactivés pendant la saisie dans un champ de texte et lorsque la visionneuse est ouverte. La navigation au clavier avec `Tab` reste disponible pour les contrôles.

IrisScope n'a pas vocation à devenir un logiciel de cabinet médical complet. Le projet évite volontairement les fonctions qui compliqueraient inutilement l'usage : comptes utilisateurs, cloud obligatoire, agenda, facturation, dossiers médicaux complexes, diagnostic automatique ou IA médicale.

## Firefly DE400 testé

Le matériel de référence actuellement diagnostiqué est :

- Infoxelle Co., Ltd. Digital Microscope / Firefly DE400 OEM ;
- USB 2.0 ;
- UVC 1.00 ;
- VID `21cd` ;
- PID `603b` ;
- résolution maximale : **1280 × 1024** ;
- formats principaux : **MJPEG** et **YUYV**.

Sur l'exemplaire testé le 1er octobre 2026, le mode le plus détaillé exposé est le **1280 × 1024 MJPEG à 8 FPS annoncés par le périphérique**. Le diagnostic du binaire à jour a reçu et décodé trois images dans ce mode ; ce court essai ne mesure pas la cadence soutenue. Le 30 FPS à cette résolution n'est pas exposé par ce matériel.

Sur l'exemplaire étudié, le bouton Snapshot n'apparaît pas comme un périphérique HID séparé. Aucun des trois backends n'émet actuellement son événement vers l'application. L'utiliser demandera l'identification du signal matériel, une implémentation propre à chaque plateforme et des essais sur appareil réel.

## Architecture technique

Le projet privilégie une pile native, légère et sans couche web :

- **Rust** pour le cœur, la concurrence, les buffers et la logique applicative ;
- **Slint** pour l'interface ;
- **V4L2** sous Linux ;
- **Media Foundation** sous Windows ;
- **AVFoundation** sous macOS ;
- traitement d'image natif et décodage MJPEG optimisé ;
- architecture caméra commune afin que l'interface et la logique restent identiques sur les trois systèmes.

Organisation du workspace :

- `iriscope-core` : abstraction caméra, capacités, sessions, stockage, bibliothèque et vidéo ;
- `iriscope-camera-linux` : backend V4L2 ;
- `iriscope-camera-windows` : backend Media Foundation ;
- `iriscope-camera-macos` : backend AVFoundation ;
- `iriscope-imaging` : décodage et transformations d'image, sans dépendance au cœur applicatif ;
- `iriscope-app` : application Slint.

Dans `iriscope-app`, `main.rs` choisit le diagnostic ou l'interface. `lib.rs`
déclare les modules et `ui.rs` inclut une seule fois l'interface Slint, réutilisée
par les tests. `gui.rs` assemble la fenêtre ; les contrôleurs caméra, capture,
patient, bibliothèque, visionneuse et réglages installent leurs callbacks.
`runtime.rs` possède l'état partagé, les mailboxes et les workers et coordonne
leur fermeture. Les workers photo, enregistrement, bibliothèque et références
sont séparés de leurs contrôleurs.

Dans le cœur, `library.rs` expose l'API de bibliothèque et ses sous-modules
séparent patients, index, scan, présentation et transactions. `file_validation.rs`
regroupe les versions et empreintes de fichiers, utilisées aussi par le lecteur
AVI. Les modules déclarent leurs dépendances par des imports explicites.

L'interface est répartie dans `ui/` : `main.slint` conserve la fenêtre et les
raccourcis, `state.slint` expose l'état partagé, `theme.slint` définit les
couleurs et espacements, `controls.slint` contient les boutons et curseurs
réutilisables, et chaque écran ou fenêtre superposée possède son propre
fichier. Les polices et leurs licences sont conservées dans `ui/fonts/`.
Cette séparation permet d'ajuster les dimensions et les composants visuels
sans parcourir un seul grand fichier.

## État d'avancement

### Linux

Le backend V4L2 est le plus avancé et sert actuellement de plateforme de référence.

Il sait notamment :

- détecter la caméra ;
- découvrir les modes et contrôles ;
- ouvrir le flux en MMAP ;
- recevoir les frames natives ;
- afficher le live ;
- capturer les images ;
- modifier les réglages caméra ;
- enregistrer le flux MJPEG ;
- alimenter la bibliothèque et les aperçus.

Les nœuds multi-plane sont également pris en charge pour les formats progressifs linéaires MJPEG, YUYV, BGRA, NV12 et NV12M. Les formats tiled, 10 bits et les nœuds de conversion mémoire-à-mémoire ne sont pas proposés. Les tests de disposition mémoire sont automatisés ; les ioctls multi-plane restent à vérifier sur un appareil correspondant. Le DE400 utilise le chemin mono-plane.

Le bouton physique du DE400 n'est pas encore pris en charge. Sur l'exemplaire branché, le pilote Linux n'expose aucun événement de bouton. L'outil `python3 scripts/diagnose-de400-button.py --seconds 45` permet d'observer les marqueurs STILL_IMAGE des métadonnées UVC pendant trois pressions espacées, après fermeture de l'aperçu IrisScope. Vérifier les chemins `--video` et `--metadata` (par défaut `/dev/video0` et `/dev/video1`) avant lancement. L'outil utilise `v4l2-ctl`, ne conserve aucune image et ne transmet aucune commande USB propriétaire. Un marqueur observé doit être corrélé aux pressions avant toute intégration automatique.

Les deux tests qui exigent un DE400 branché restent ignorés par les tests automatisés ordinaires.

### Windows

Le backend Media Foundation sait :

- énumérer les caméras ;
- ouvrir le périphérique ;
- découvrir les media types natifs ;
- sélectionner un mode ;
- recevoir les frames via Media Foundation ;
- participer au suivi connexion / déconnexion ;
- alimenter le même pipeline photo, vidéo et bibliothèque que les autres plateformes.

Les contrôles d'image utilisent les interfaces IAM lorsqu'elles sont exposées et un repli standard IKsControl sur la même source caméra. Seules les plages et les modes réellement déclarés par le pilote sont présentés. La réinitialisation utilise la valeur native par défaut, ou la valeur constatée à l'ouverture lorsque le pilote n'annonce pas de défaut. Les tests physiques avec le DE400 sous Windows et l'intégration du bouton Snapshot restent à réaliser.

### macOS

Le backend AVFoundation sait :

- énumérer et ouvrir les caméras ;
- démarrer le flux vidéo natif ;
- recevoir les frames décodées BGRA/NV12 ou les échantillons MJPEG selon le mode fourni ;
- demander une sortie native pour les modes MJPEG afin de conserver les JPEG caméra sans réencodage ;
- participer au suivi connexion / déconnexion ;
- alimenter le même pipeline photo, vidéo et bibliothèque que les autres plateformes.

Les contrôles AVFoundation actuellement exposés se limitent aux modes de balance des blancs automatique et d'exposition lorsque la caméra les fournit. Les tests physiques avec le DE400 et l'intégration du bouton Snapshot restent à réaliser.

## Principes de performance

Le projet donne la priorité à la qualité d'image et à la faible latence :

- pas d'Electron ;
- pas de WebView dans le chemin vidéo ;
- pas de conversion d'image inutile ;
- conservation du JPEG natif quand il existe ;
- encodage JPEG vidéo seulement lorsque le backend ne livre pas de MJPEG ;
- buffers bornés pour éviter l'accumulation de retard ;
- priorité à la dernière frame reçue ;
- rotation et miroir du live appliqués au rendu plutôt qu'en recopiant la frame sur le CPU ;
- séparation entre capture originale et transformations d'affichage ;
- backends caméra natifs pour chaque système.

Les queues photo et vidéo ont chacune un budget de 64 Mio en plus de leur
limite de nombre d'éléments. Les backends refusent les frames MJPEG au-delà de
32 Mio, les frames brutes au-delà de 64 Mio et les résolutions supérieures à
8192 pixels par côté ou 16 mégapixels. Les miniatures empruntent le buffer
source au lieu de recopier l'image entière avant redimensionnement. Le worker
de références regroupe les demandes et abandonne les résultats dépassés.

Le scan initial trie les métadonnées ; la vérification SHA-256 est ciblée sur
les fichiers affichés ou nécessaires au filtre patient. Une association avec
un dossier exige toujours une vérification du contenu. Les scans et calculs
d'empreinte acceptent l'annulation, et la fermeture partage un budget d'attente
entre les workers. Les publications engagées conservent leur journal de reprise.
Le paquet de distribution et son empreinte sont également produits en flux.

La publication des petits fichiers (au plus 1 Mio) relit leur contenu au commit.
Les médias plus grands attendent la stabilité de leur version hors verrou
d'index avant le dernier SHA, ce qui peut ajouter environ une seconde à la
finalisation. Sur les systèmes de fichiers sans version fiable, la validation
conserve une relecture complète sous verrou. L'annulation couvre aussi la
migration des empreintes d'anciens index ; la reprise d'un journal et le commit
atomique se terminent en conservant leur protocole de durabilité.

## Validation locale

### Distribution portable

Sur chaque système cible, construire puis empaqueter le binaire natif :

```text
cargo build --release -p iriscope-app --locked
python scripts/package-release.py
```

Le script crée dans `dist/` une archive et son empreinte SHA-256. Vérifier
l'empreinte depuis ce dossier avec `cd dist && sha256sum -c *.sha256` sur Linux.
Sous Linux,
l'archive `.tar.gz` contient le programme `iriscope-app` à exécuter après
extraction. Sous Windows, l'archive `.zip` contient `IrisScope.exe`. Sous
macOS, le `.zip` contient `IrisScope.app` avec la déclaration d'autorisation
caméra nécessaire à AVFoundation. Les licences des polices embarquées sont
jointes à chaque archive. La CI produit ces trois formats sur leurs systèmes
respectifs et les conserve comme artefacts téléchargeables.

Ces archives sont portables : elles ne créent pas de raccourci ni de mise à jour
automatique. Elles ne sont ni signées ni notarisées. Pour une distribution
publique avec installation système, il reste à prévoir la signature, les
installateurs et un processus de mise à jour adapté à chaque plateforme. Le
paquet Linux est construit sur Ubuntu 24.04 et dépend des bibliothèques système
requises par Slint et V4L2 ; sa compatibilité avec d'autres distributions doit
être vérifiée séparément.

### Développement et tests

Pour lancer l'application et juger la latence du live dans les conditions de
distribution :

```text
cargo run --release -p iriscope-app
```

Le profil de développement optimise également les crates de décodage d'image,
mais le profil `release` reste la référence pour les mesures de performance.

```text
./scripts/ci-local.sh
```

Pour inclure le diagnostic de la caméra branchée :

```text
./scripts/ci-local.sh --hardware
```

Cette variante échoue si la DE400 est absente ou si elle ne fournit pas trois
images décodables.

Le script contrôle Linux nativement et vérifie la compilation des cibles Windows x86_64,
macOS Intel et macOS Apple Silicon si les cibles Rust correspondantes sont installées.
Clippy compile tous les targets en même temps qu'il les analyse ; une seconde
passe `cargo check` identique n'est donc pas lancée. Le script utilise un seul
job par défaut pour limiter la mémoire.
La CI GitHub exécute aussi les tests et l'analyse statique sur des machines Linux,
Windows et macOS. Ces vérifications ne remplacent pas les essais avec un DE400 branché
sur chaque système.
