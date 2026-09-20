# IrisScope

IrisScope est une application desktop multiplateforme destinée à l'utilisation
d'un iridoscope Firefly DE400 / Infoxelle Digital Microscope.

## Architecture

Le projet partage le cœur métier et l'interface entre les trois plateformes :

- `iriscope-core` : abstraction caméra, capacités, sessions, stockage, bibliothèque et vidéo ;
- `iriscope-camera-linux` : backend V4L2 ;
- `iriscope-camera-windows` : backend Media Foundation ;
- `iriscope-camera-macos` : backend AVFoundation ;
- `iriscope-imaging` : décodage MJPEG/YUYV et transformations d'image ;
- `iriscope-renderer` : état de présentation du flux ;
- `iriscope-app` : application Slint.

## État actuel

### Linux

Le backend V4L2 sait énumérer les caméras, découvrir leurs modes et contrôles,
ouvrir un flux MMAP et livrer les frames natives à l'application. Le live,
la capture JPEG, les réglages caméra et l'enregistrement MJPEG AVI sont reliés
à l'interface.

### Windows

Le backend Media Foundation sait énumérer les périphériques, ouvrir le DE400 et
découvrir les media types natifs. Le streaming, les contrôles et le hotplug
restent à finaliser et à tester physiquement.

### macOS

Le backend AVFoundation sait énumérer les périphériques et leurs formats.
Le streaming, les contrôles et le hotplug restent à finaliser et à tester
physiquement.

## Fonctionnalités applicatives déjà présentes

- interface Slint ;
- session prénom / nom / œil ;
- nommage automatique des captures ;
- sauvegarde de la frame MJPEG native avec ajout DHT si nécessaire ;
- mode vidéo MJPEG AVI sans réencodage ;
- bibliothèque avec anonymisation des anciennes sessions ;
- freeze, transformations d'affichage et réglages caméra ;
- diagnostic du flux (format, résolution, FPS mesuré, temps de décodage).

## Validation locale

```text
./scripts/ci-local.sh
```

Pour inclure le diagnostic de la caméra branchée :

```text
./scripts/ci-local.sh --hardware
```

La CI GitHub compile et teste également nativement Linux, Windows et macOS.
