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

## Fonctions prévues

### Caméra et live

- connexion automatique au Firefly DE400 ;
- affichage live dès l'ouverture de l'application ;
- mode **Photo** ou **Vidéo** clairement sélectionnable ;
- gros bouton logiciel de capture ;
- bouton physique du DE400 utilisable pour déclencher la capture ;
- en mode vidéo, le même bouton démarre puis arrête l'enregistrement ;
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

- enregistrement depuis le flux du DE400 ;
- priorité à une solution évitant le réencodage lorsque le flux MJPEG natif est disponible ;
- démarrage / arrêt depuis l'interface ou le bouton physique ;
- classement dans la même bibliothèque que les photos ;
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

La bibliothèque prévoit :

- photos et vidéos réunies au même endroit ;
- miniatures ;
- recherche par personne ;
- regroupement par personne et/ou date ;
- visualisation d'une photo dans l'application ;
- ouverture des fichiers enregistrés ;
- accès rapide au dossier de stockage ;
- anonymisation des anciennes sessions lorsque nécessaire.

Le stockage reste local à l'ordinateur. Aucun compte ou cloud n'est requis.

### Réglages de l'image

Les réglages sont accessibles dans un panneau secondaire afin de ne pas surcharger l'écran principal.

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

Une page **Réglages** doit permettre notamment de consulter ou modifier :

- état de connexion du DE400 ;
- mode par défaut Photo / Vidéo ;
- action du bouton physique ;
- dossier d'enregistrement ;
- modèle de nommage ;
- réglages caméra ;
- thème de l'application.

## Interface volontairement simple

L'application doit rester limitée à quatre zones principales :

1. **Caméra**
2. **Bibliothèque**
3. **Références iris**
4. **Réglages**

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
- recevoir les frames via Media Foundation.

Les tests physiques avec le DE400, les contrôles caméra, le hotplug et le bouton physique restent à finaliser.

### macOS

Le backend AVFoundation est en cours de raccordement complet au même pipeline de frames que Linux et Windows.

Les tests physiques sur Mac avec le DE400, les contrôles caméra, le hotplug et le bouton physique restent à finaliser.

## Principes de performance

Le projet donne la priorité à la qualité d'image et à la faible latence :

- pas d'Electron ;
- pas de WebView dans le chemin vidéo ;
- pas de conversion d'image inutile ;
- conservation du JPEG natif quand il existe ;
- buffers bornés pour éviter l'accumulation de retard ;
- priorité à la dernière frame reçue ;
- séparation entre capture originale et transformations d'affichage ;
- backends caméra natifs pour chaque système.

## Validation locale

```text
./scripts/ci-local.sh
```

Pour inclure le diagnostic de la caméra branchée :

```text
./scripts/ci-local.sh --hardware
```

La CI GitHub compile et teste le projet nativement sous Linux, Windows et macOS.
