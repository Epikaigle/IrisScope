# IrisScope

IrisScope est une application desktop native et multiplateforme conçue pour utiliser simplement un **iridoscope Firefly DE400 / Infoxelle Digital Microscope** sous **Linux, Windows et macOS**.

L'objectif est de remplacer l'ancien logiciel FireflyPro par une application moderne, rapide et très simple à utiliser : on branche le DE400, l'image apparaît immédiatement, on renseigne la personne examinée et l'œil, puis on capture les photos ou vidéos. IrisScope s'occupe automatiquement du nommage, du classement et de la bibliothèque.

> IrisScope est un logiciel de capture, de visualisation et d'organisation d'images. Les planches d'iridologie intégrées sont des références visuelles et ne constituent pas un outil de diagnostic médical.

## Expérience utilisateur visée

L'écran **Caméra** est l'écran principal et doit suffire pour la grande majorité de l'utilisation.

Au lancement, IrisScope doit :

1. détecter automatiquement le Firefly DE400 ;
2. ouvrir le meilleur flux vidéo disponible sans demander de choisir une caméra ;
3. afficher immédiatement le live ;
4. permettre de renseigner le prénom / nom de la personne ;
5. permettre de choisir **œil gauche** ou **œil droit** ;
6. permettre de prendre une photo ou de démarrer / arrêter une vidéo depuis l'interface ou avec le bouton physique du DE400 ;
7. enregistrer et classer automatiquement le fichier ;
8. rendre immédiatement la capture accessible dans la bibliothèque.

Aucune étape technique de type « ouvrir la caméra », « choisir /dev/video0 » ou « sélectionner un codec » ne doit être nécessaire en usage normal.

## Fonctions disponibles et en cours

### Caméra et live

- connexion automatique au Firefly DE400 ;
- affichage live dès l'ouverture de l'application ;
- mode **Photo** ou **Vidéo** clairement sélectionnable ;
- gros bouton logiciel de capture ;
- abstraction prévue pour le bouton physique du DE400 ; son intégration fiable reste à finaliser selon la plateforme ;
- comportement du bouton configurable : suivre le mode courant, toujours Photo ou toujours Vidéo ;
- indicateur **REC** et durée d'enregistrement ;
- dernière capture visible immédiatement ;
- détection de déconnexion / reconnexion de la caméra ;
- conservation d'une latence minimale grâce à une stratégie « dernière frame disponible ».

### Photos

- capture à la résolution native maximale disponible ;
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
- encodage JPEG uniquement lorsque le backend fournit du YUYV/BGRA, notamment sur macOS ;
- démarrage / arrêt depuis l'interface ;
- finalisation propre du fichier en cas de déconnexion de la caméra ;
- classement dans la même bibliothèque que les photos ;
- lecteur vidéo privé intégré avec lecture / pause, position, durée et navigation dans la timeline ;
- cadence de lecture compensée par le temps de décodage pour rester proche du FPS enregistré ;
- affichage du temps d'enregistrement.

### Session et nommage

La saisie doit rester volontairement minimale :

- prénom ;
- nom ;
- œil gauche / droit.

Ces informations restent actives jusqu'à ce que l'utilisateur les change afin de pouvoir réaliser plusieurs captures successives rapidement.

Les fichiers sont automatiquement nommés avec :

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
- filtres Tous / Photos / Vidéos / Patient sélectionné ;
- ouverture initiale sur toutes les captures, puis conservation du filtre et de la page entre les onglets ;
- anonymisation du titre des captures appartenant à d'autres personnes ;
- index local caché de métadonnées, indépendant du modèle de nom de fichier ;
- visionneuse photo privée intégrée avec zoom / déplacement ;
- lecteur vidéo MJPEG privé intégré avec timeline et recherche dans la vidéo ;
- accès rapide au dossier de stockage ;
- accès à la carte d'iridologie depuis la bibliothèque, dans une fenêtre refermable.

Le stockage reste local à l'ordinateur. Aucun compte ou cloud n'est requis.

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

IrisScope doit proposer un accès rapide à des images de référence :

- carte / planche de l'iris ;
- références œil gauche / œil droit ;
- planche de signes / symboles ;
- zoom et déplacement dans les images.

Ces éléments sont uniquement des aides visuelles de consultation.

### Réglages généraux

La page **Réglages** permet notamment de consulter ou modifier :

- état de connexion et diagnostic du DE400 ;
- action prévue du bouton physique ;
- dossier d'enregistrement ;
- modèle de nommage, avec `{prenom}`, `{nom}` et `{oeil}` obligatoires ;
- chemins des deux images de référence d'iridologie.

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

Les raccourcis de capture exigent un flux actif, un prénom, un nom et un œil sélectionné. Ils sont désactivés pendant la saisie dans un champ de texte et lorsque la visionneuse est ouverte. La navigation au clavier avec `Tab` reste disponible pour les contrôles.

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

Sur l'exemplaire testé, le meilleur mode réel pour la qualité est le **1280 × 1024 MJPEG**, mesuré autour de **6,25 FPS**. Le 30 FPS à cette résolution n'est pas exposé par ce matériel.

Le bouton Snapshot n'est pas un périphérique HID séparé : son événement est transmis via l'interface UVC de la caméra. Son intégration native est donc gérée séparément selon la plateforme.

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
- `iriscope-imaging` : décodage et transformations d'image ;
- `iriscope-renderer` : état de présentation du flux ;
- `iriscope-app` : application Slint.

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

La récupération propre du bouton physique UVC du DE400 est encore en cours de finalisation.

### Windows

Le backend Media Foundation sait :

- énumérer les caméras ;
- ouvrir le périphérique ;
- découvrir les media types natifs ;
- sélectionner un mode ;
- recevoir les frames via Media Foundation ;
- participer au suivi connexion / déconnexion ;
- alimenter le même pipeline photo, vidéo et bibliothèque que les autres plateformes.

Les tests physiques avec le DE400 et l'intégration du bouton Snapshot restent à finaliser.

### macOS

Le backend AVFoundation sait :

- énumérer et ouvrir les caméras ;
- démarrer le flux vidéo natif ;
- recevoir les frames BGRA/YUYV ;
- participer au suivi connexion / déconnexion ;
- alimenter le même pipeline photo, vidéo et bibliothèque que les autres plateformes.

Les tests physiques avec le DE400 et l'intégration du bouton Snapshot restent à finaliser.

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

## Validation locale

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

Toutes les vérifications sont exécutées localement sur la machine de développement.
Le script contrôle Linux nativement et compile aussi les cibles Windows x86_64,
macOS Intel et macOS Apple Silicon. Les essais matériels Windows et macOS restent
à exécuter sur les machines correspondantes.
