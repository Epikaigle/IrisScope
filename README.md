# IrisScope

IrisScope est une application desktop native et multiplateforme conçue pour utiliser simplement un **iridoscope Firefly DE400 / Infoxelle Digital Microscope** sous **Linux, Windows et macOS**.

L'objectif est de remplacer l'ancien logiciel FireflyPro par une application moderne, rapide et très simple à utiliser : on branche le DE400, l'image apparaît immédiatement, on renseigne la personne examinée et l'œil, puis on capture les photos ou vidéos. IrisScope s'occupe automatiquement du nommage, du classement et de la bibliothèque.

> IrisScope est un logiciel de capture, de visualisation et d'organisation d'images. L'utilisateur peut ajouter ses propres images de référence d'iridologie ; aucune planche n'est fournie avec l'application. Ces images sont uniquement des références visuelles et ne constituent pas un outil de diagnostic médical.

Le [site de présentation](https://github.com/Epikaigle/IrisScope/tree/main/site) contient une vue 3D du DE400 et des
captures de l’interface. Le [guide GitHub Pages](https://github.com/Epikaigle/IrisScope/blob/main/docs/SITE.md) explique comment
le publier manuellement depuis `main`, sans branche supplémentaire.

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
- bouton physique du DE400 sous Linux via la [passerelle locale](helpers/linux/README.md) ;
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

Un enregistrement est limité à une heure et à la capacité du conteneur AVI classique (environ 4 Gio). Une interruption trop longue de la caméra arrête l'enregistrement avec un message et conserve le fichier temporaire récupérable.

### Session et nommage

La saisie reste volontairement minimale :

- prénom et nom pour une capture associée à un dossier ;
- œil gauche / droit.

Une capture anonyme reste possible sans prénom ni nom. Dès qu'un des deux champs est renseigné, les deux doivent l'être pour choisir un dossier. Aucun autre renseignement personnel n'est demandé.

Pendant la saisie, IrisScope propose les dossiers dont les noms correspondent. L'utilisateur choisit **Choisir** pour retrouver une personne déjà enregistrée ou **Créer une autre personne** pour un homonyme. Deux dossiers peuvent donc porter exactement les mêmes prénom et nom : chacun reçoit un numéro stable, par exemple `D-000042`. La date de dernière capture apparaît dans les résultats lorsqu'elle est disponible. L'application ne choisit et ne fusionne jamais deux homonymes automatiquement. Le numéro du dossier sélectionné reste visible pendant la session et les nouvelles captures lui sont associées ; il suffit de choisir une fois le dossier pour une série de prises de vue.

Pour revoir un patient, saisir son prénom et son nom puis sélectionner le bon numéro, ou saisir directement son numéro dans le champ « Numéro de dossier ». Cette recherche directe reste disponible même s'il existe de nombreux homonymes. En cas de doute, consulter ses captures dans la bibliothèque avant d'enregistrer de nouvelles images. Le numéro de dossier est l'identifiant fiable : les noms servent à la recherche. Les captures anciennes qui n'ont pas de numéro restent sans dossier jusqu'à une attribution explicite.

Le dossier et l'œil restent actifs jusqu'à ce que l'utilisateur les change afin de pouvoir réaliser plusieurs captures successives rapidement. Utiliser **Terminer session** avant de commencer avec une autre personne.

Les boutons **Gauche / Droit** sélectionnent uniquement l’œil examiné, sans compteur dans leur libellé. Le changement d’œil reste manuel.

Les fichiers sont automatiquement nommés avec, lorsqu'ils sont renseignés :

- le nom de la personne ;
- le côté de l'œil ;
- la date ;
- l'heure.

### Bibliothèque

IrisScope doit permettre de retrouver les captures sans devoir parcourir les dossiers du système.

La bibliothèque propose actuellement :

- photos et vidéos réunies au même endroit ;
- miniatures pour les photos et pour les vidéos AVI à partir de leur première image, chargées progressivement pour les cartes visibles pendant le défilement ;
- affichage au choix en grille ou en liste, avec sélection puis ouverture dans la visionneuse ;
- filtres Tous / Photos / Vidéos / Dossier actif, recherche par numéro de dossier, œil et période inclusive, champs **Du / Au** au format JJ/MM/AAAA avec calendrier contextuel, sans masquer la page ; le format AAAA-MM-JJ reste accepté ; tri des plus récentes ou des plus anciennes ;
- ouverture initiale sur toutes les captures, puis conservation du filtre et de la page entre les onglets ;
- masquage du nom dans le titre des captures appartenant à d'autres personnes ; les noms de fichiers et les miniatures restent visibles dans le dossier de stockage ;
- index local caché de métadonnées et de numéros de dossier, indépendant du modèle de nom de fichier ;
- visionneuse photo intégrée : zoom précis de 10 à 800 %, taille réelle en pixels, loupe, notes et annotations, références et comparaison de photos ;
- lecteur vidéo MJPEG AVI intégré avec timeline et recherche dans la vidéo ;
- ouverture des autres vidéos détectées (par exemple MP4 ou MKV) avec l'application associée du système ;
- accès rapide au dossier de stockage ;
- pagination de 100 captures, avec réutilisation du tri et des filtres entre les pages ; les fichiers visibles sont revérifiés avant affichage, et **Actualiser** force une relecture complète ;
- accès à la carte d'iridologie depuis la bibliothèque, dans une fenêtre refermable.

**Dossiers / Séances**, dans la bibliothèque, recherche une personne par nom ou numéro. Choisir explicitement le bon dossier ouvre ses journées de consultation, avec les photos gauche/droite et des notes générales distinctes des notes de chaque photo. La journée actuelle est disponible même avant la première capture ; les journées sont regroupées par date, sans agenda de rendez-vous. **Séances**, dans la zone Patient, ouvre directement le dossier actif. **Utiliser pour la capture** sélectionne le dossier pour de nouvelles prises.

Les notes générales sont sauvegardées automatiquement après une pause de 700 ms, avec un état visible et l’heure du dernier enregistrement. Une erreur garde le brouillon pour réessayer ou le copier. Jusqu’à 200 photos récentes par journée sont affichées avec leurs miniatures ; les vidéos et captures sans œil renseigné restent accessibles dans la bibliothèque. Les notes et les marques **Retenue** font partie de la sauvegarde locale.

Le stockage reste local à l'ordinateur. Aucun compte ou cloud n'est requis. Les
fichiers, leurs noms, les miniatures affichées et l'index de métadonnées ne
sont pas chiffrés par IrisScope : les personnes ayant accès au dossier de
captures peuvent consulter ces informations. Le masquage des titres dans la
bibliothèque est une aide visuelle, pas une protection d'accès.

Les dossiers patients sont organisés dans la bibliothèque par numéro stable
(`D-000042`), puis consultables par date, œil et type de capture. Sur disque,
les médias restent ensemble dans le dossier de stockage sélectionné ;
`.iriscope-index.json` conserve les liens entre fichiers et dossiers patients.
Deux homonymes restent donc séparés même si leurs noms sont identiques. Les noms
de fichiers indiquent prénom, nom, œil, date et heure avec le modèle par défaut ;
des prises simultanées reçoivent un suffixe `_2`, `_3`, etc., sans écrasement.
Une capture anonyme conserve le préfixe `Iris` et l'œil choisi. Cette organisation
est conservée par la sauvegarde/restauration intégrée.

Dans la visionneuse photo, **Ajuster** affiche toute l’image et **100 %** affiche un pixel photo par pixel écran, en tenant compte de l’échelle du moniteur. La molette zoome sur le curseur, les boutons −/+ et le curseur règlent le zoom, le glisser et les flèches déplacent l’image. Un double-clic rétablit l’image entière. **Loupe** examine une zone sans changer le cadrage et **Plein écran** agrandit la fenêtre ; Échap quitte d’abord le plein écran, puis ferme la photo.

Les panneaux **Annoter / Notes**, **Références**, **Affichage** et **Comparer** se referment pour rendre la place à la photo. **Exporter…** ouvre séparément les commandes de copie. Les notes et les cercles, flèches, points ou textes sont enregistrés automatiquement après une pause dans la saisie, avec date de modification et empreinte de la photo dans l’index privé de la bibliothèque. Les annotations utilisent les coordonnées de l’original, restent positionnées après une rotation ou un miroir, peuvent être masquées, retirées ou annulées. Navigation et fermeture attendent la fin de l’enregistrement ; en cas d’échec, le brouillon reste ouvert pour réessayer ou le copier avant un abandon explicite. Les sauvegardes/restaurations comprennent ces observations. Une photo remplacée ne récupère pas les annotations de l’ancien contenu.

**Comparer** recherche les photos du même dossier et du même œil, avec des dates facultatives et jusqu’à 200 résultats récents par recherche. Le choix affiche des miniatures, la date, l’œil et les photos marquées **Retenue**. **Retenir**, dans la visionneuse, conserve cette marque entre les ouvertures sans modifier l’original. Les deux vues peuvent être positionnées indépendamment, puis liées en conservant leur alignement. **Garder cette photo pour comparer** conserve aussi l’original comme référence : ouvrir une autre photo ou utiliser **Suivante** l’affiche à côté, avec les dates, yeux et dossiers. **Effacer la comparaison** revient à une seule image. Changer de dossier de stockage efface cette référence.

**Références** affiche la carte ou la planche importée à côté de la photo, avec zoom et déplacement. Les cercles repères facultatifs se centrent manuellement. **Affichage** règle luminosité, contraste, rotation et miroir ; le bouton **Rotation** ouvre un curseur de 0 à 359° et une saisie au degré près, également disponibles dans le direct. Les annotations suivent la rotation et le miroir dans la copie PNG. **Original** permet de comparer et **Réinitialiser l’affichage** remet ces réglages à zéro. Ils ne sont pas enregistrés dans la photo et reviennent à l’original à chaque ouverture.

**Copie PNG annotée…** exporte l’affichage avec les annotations. **Fiche PDF et notes…** exporte l’original annoté, sa légende et les notes, avec la deuxième photo lorsque la comparaison est active. Les notes longues continuent sur plusieurs pages. Les exports conservent les sources et refusent de remplacer un fichier existant. Le détail des outils et de leurs limites figure dans [Visionneuse photo](docs/VISIONNEUSE.md).

Les paramètres proposent **Sauvegarder** et **Restaurer…**. La sauvegarde crée un nouveau dossier complet, incluant les fichiers cachés et les vidéos interrompues, à l’extérieur du dossier des captures. Elle vérifie les fichiers pendant la copie et publie le résultat après sa finalisation. La restauration contrôle les empreintes de tous les fichiers puis crée et ouvre un nouveau dossier, sans remplacer les captures actuelles. Les liens symboliques, sauvegardes incomplètes, chemins invalides et versions inconnues sont refusés. Les sauvegardes ne sont pas chiffrées.

La progression indique les fichiers et le volume copiés ; **Annuler** reste accessible
pendant la copie et l’attente des verrous. Une annulation retire les fichiers temporaires.
Après une interruption brutale, un dossier caché `.en-cours` peut rester sur le support :
il est refusé comme sauvegarde restaurable. Une nouvelle sauvegarde reste possible.
Les paramètres affichent la dernière sauvegarde réussie du dossier actuellement choisi.
Un rappel facultatif, désactivé par défaut, apparaît après sept jours sans sauvegarde.

Dans la visionneuse vidéo, **Exporter MP4…** crée une copie H.264 pour le partage,
avec progression et annulation. L’AVI original est conservé et un fichier existant
n’est jamais remplacé. Cette fonction utilise FFmpeg avec `libx264`, installé sur le
système ou inclus dans un paquet. Sans FFmpeg, les captures et la lecture AVI restent disponibles.

L’espace disponible est affiché dans les paramètres et revérifié régulièrement. Une alerte apparaît sous 1 Gio ; les nouvelles captures sont bloquées sous la réserve de 64 Mio. Les workers vérifient aussi l’espace avant l’écriture d’une photo ou le démarrage d’une vidéo. Une vidéo reste arrêtable et s’arrête lorsque l’espace devient critique. Cette réserve ne garantit pas qu’un autre programme n’occupera pas le disque entre deux vérifications.

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

Aucune image de référence n'est livrée avec le logiciel : la recherche n'a pas trouvé de paire cartographie/symboles offrant à la fois une qualité adaptée et une autorisation explicite de redistribution. Chacun peut choisir dans **Paramètres** des images qu'il est autorisé à utiliser. Ces éléments sont uniquement des aides visuelles de consultation.

### Réglages généraux

La page **Paramètres** permet notamment de consulter ou modifier :

- état de connexion et diagnostic du DE400 ;
- dossier d'enregistrement ;
- modèle de nommage avec les jetons `{prenom}`, `{nom}` et `{oeil}` obligatoires dans le modèle ; les champs patient peuvent rester vides lors d'une capture, et `{date}` / `{heure}` sont facultatifs ;
- qualité du flux vidéo et thème clair, sombre ou selon le système ;
- chemins des deux images de référence d'iridologie.

Les images de référence sont limitées à 32 Mio sur disque et 16 mégapixels au décodage pour éviter une allocation excessive en mémoire.

Les contrôles image réellement exposés par le backend sont générés dynamiquement dans la fenêtre **Réglages image** de la caméra. Les menus permettent de choisir directement une valeur dans une liste. Les libellés tronqués sont consultables au survol.

**Parcourir…** permet de choisir le dossier des captures et les images de référence avec le sélecteur du système. La saisie manuelle du chemin reste disponible.
Sous Linux, le sélecteur utilise le portail du bureau, ou `zenity` en repli.

## Interface et affichage

**Paramètres → Interface et affichage** regroupe le thème, la mémorisation de la disposition, l’ouverture en **Image seule** et la taille des miniatures. Les panneaux patient, photo et dossiers sont redimensionnables avec des bornes adaptées à la fenêtre. Les préférences retrouvent les panneaux et la vue grille/liste au redémarrage ; **Réinitialiser la disposition** conserve les captures et les notes.

La bibliothèque et les paramètres proposent les préréglages **Petites / Moyennes / Grandes** et un curseur de **80 à 480 pixels**, par pas d'un pixel logique, conservé au redémarrage. La grille adapte le nombre de colonnes à cette largeur. Les menus se modifient par sélection explicite ou au clavier : la molette sur un menu fermé fait défiler la page sans changer sa valeur.

**Présenter**, dans les outils du direct ou de la visionneuse, affiche l'iris en plein écran, masque les noms, dossiers, fichiers, notes et annotations à l’écran et bloque l’accès aux vues privées jusqu’à **Fin présentation**. La sortie rétablit l'onglet, le plein écran et le panneau précédents : un panneau masqué reste masqué. L’œil, les erreurs et les commandes de capture restent accessibles. **Image seule** masque les barres photo après trois secondes : mouvement de souris, **Commandes** et Échap assurent le retour. Le diagnostic technique se déplie avec **Détails**. L’aide existante reste disponible.

## Interface volontairement simple

L'application comporte trois espaces principaux :

1. **Caméra**
2. **Bibliothèque**
3. **Paramètres et diagnostic**

Le panneau patient est à gauche de l'image et peut être masqué pour agrandir l'aperçu. Le formulaire défile séparément et le bouton de capture reste accessible en bas. Les outils d'image sont regroupés sous le direct, sur une ou deux lignes selon la largeur ; **Réglages** ouvre les contrôles de la caméra. Le réglage d’angle s’ouvre au-dessus de ces commandes. La molette et le curseur de zoom restent synchronisés dans le direct et dans les images de référence. Les images de référence s'ouvrent depuis **Références** dans la bibliothèque. Les paramètres et leur diagnostic s'empilent dans les fenêtres étroites. La taille minimale est de 800 × 600 pixels.

En plein écran, la capture reste en bas et **Quitter le plein écran** en haut à
droite. Ces commandes réapparaissent au mouvement de la souris ou avec `Tab`,
puis disparaissent après trois secondes d'inactivité. Elles restent visibles
pendant le survol, un clic maintenu ou leur utilisation au clavier. Une capture
indisponible indique la condition manquante. `Échap` permet de revenir, même
après un clic dans l'image. Les commandes de lecture et la timeline des vidéos
enregistrées restent sous l'image dans la visionneuse.

Les commandes et informations sur l'image partagent la même police, le même
fond sombre et les mêmes arrondis. Les menus et leurs options s’alignent à gauche, avec une flèche centrée et des lignes espacées. Les filtres de dossier et de dates gardent une largeur compacte ; **Calendrier** ouvre le sélecteur de date. **Dossiers et séances** recherche un patient puis une date ; **Afficher les fichiers** ouvre le dossier sur le disque. Les messages de capture restent dans la zone vidéo et leurs actions
s'alignent au centre, même avec un nom de fichier sur plusieurs lignes.

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
| `Échap` | Fermer la visionneuse, la carte, les réglages d'image ou quitter le plein écran |

Les raccourcis numériques acceptent aussi `Ctrl+Maj+1` à `Ctrl+Maj+5`, notamment sur les claviers AZERTY. Les raccourcis de capture exigent un flux actif et un œil sélectionné. Les captures sans nom sont autorisées et reçoivent un nom de fichier commençant par `Iris`. Les raccourcis sont désactivés pendant la saisie dans un champ de texte et lorsque la visionneuse ou les références sont ouvertes. Une opération patient bloque une nouvelle capture, tout en permettant d'arrêter une vidéo en cours.

`Tab` parcourt les contrôles avec un focus visible. `Espace` ou `Entrée` sélectionne une capture de la bibliothèque. Après un clic sur une image agrandie, les flèches déplacent la vue ; **Ajuster** recentre l'image à 100 %.

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

Sur l'exemplaire étudié, le bouton Snapshot n'apparaît pas comme un périphérique HID séparé. Le 6 octobre 2026, trois pressions réelles ont été corrélées aux paquets USB `02 01 00 01`, suivis de `02 01 00 00` au relâchement, sur l'endpoint `0x81`. Sous Linux, une passerelle locale transmet ces événements au backend V4L2. Les réceptions Windows et macOS sont intégrées au code, mais nécessitent encore une validation physique sur chacun de ces systèmes.

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

`helpers/linux/` contient la passerelle du bouton DE400, son service et sa
documentation. `scripts/install-de400-button.sh` et
`scripts/uninstall-de400-button.sh` gèrent uniquement cette installation système.
Dans le backend Linux, `button.rs` reçoit les événements ; `platform.rs` conserve
la gestion V4L2 du flux et des contrôles. `hardware_validation.rs` regroupe les
essais sur caméra réelle avec des dossiers fictifs et un stockage isolé.

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
réutilisables, `combo-box.slint` les menus à sélection explicite et
`thumbnail-size.slint` le réglage commun des miniatures. Chaque écran ou fenêtre superposée possède son propre
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

Le DE400 testé déclare `bTriggerUsage = 0` : le pilote Linux ne crée pas de touche caméra pour son bouton. Ses pressions transmettent cependant des événements sur le canal USB d'interruption. La [passerelle Linux](helpers/linux/README.md) lit uniquement ces événements pour le DE400 `21cd:603b`, sans ouvrir de flux vidéo, modifier la caméra ni enregistrer d'images. Son programme installé appartient à root ; IrisScope conserve les droits habituels de l'utilisateur. L'installation requiert une authentification administrateur et le compte doit appartenir au groupe `video`.

```sh
sudo bash scripts/install-de400-button.sh
```

Dans les paramètres, le bouton peut suivre le mode **Photo / Vidéo**, toujours
prendre une photo ou toujours démarrer/arrêter une vidéo. Une pression maintenue
ne déclenche qu'une fois. Une vidéo en cours peut toujours être arrêtée lorsque
le bouton suit le mode, même si le mode affiché a changé. Le bouton logiciel reste
utilisable avec ou sans passerelle.

Pour retirer uniquement la passerelle : `sudo bash scripts/uninstall-de400-button.sh`.
Les captures et les paramètres sont conservés.

Le diagnostic `scripts/diagnose-de400-button.py` reste disponible pour les
métadonnées UVC. Sur l'exemplaire testé, les pressions ne produisent pas de
marqueur STILL_IMAGE. `sudo python3 scripts/diagnose-de400-usb-status.py` observe
le canal du bouton pendant 45 secondes ; appuyer après `STATUS_ARMED`.

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

Les contrôles d'image utilisent les interfaces IAM lorsqu'elles sont exposées et un repli standard IKsControl sur la même source caméra. Seules les plages et les modes réellement déclarés par le pilote sont présentés. La réinitialisation utilise la valeur native par défaut, ou la valeur constatée à l'ouverture lorsque le pilote n'annonce pas de défaut. Le bouton Snapshot dispose d'un abonnement natif optionnel au déclencheur UVC exposé par le pilote. Son fonctionnement avec le DE400 réel reste à vérifier ; la compilation seule ne le confirme pas. Voir [Bouton multiplateforme](docs/BOUTON-MULTIPLATEFORME.md).

### macOS

Le backend AVFoundation sait :

- énumérer et ouvrir les caméras ;
- démarrer le flux vidéo natif ;
- recevoir les frames décodées BGRA/NV12 ou les échantillons MJPEG selon le mode fourni ;
- demander une sortie native pour les modes MJPEG afin de conserver les JPEG caméra sans réencodage ;
- participer au suivi connexion / déconnexion ;
- alimenter le même pipeline photo, vidéo et bibliothèque que les autres plateformes.

Les contrôles AVFoundation actuellement exposés se limitent aux modes de balance des blancs automatique et d'exposition lorsque la caméra les fournit. Le bouton Snapshot dispose d’un récepteur USB natif optionnel ; son état réel est affiché dans le diagnostic après ouverture de la caméra. Le pilote Apple peut refuser l’accès à l’interface du bouton : le direct et les captures logicielles continuent alors de fonctionner. La compilation Mac Intel/Apple Silicon est vérifiable depuis Linux ; la réception réelle du bouton doit encore être testée sur le Mac. Voir [Bouton multiplateforme](docs/BOUTON-MULTIPLATEFORME.md).

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

### Captures sur l'appareil réel

Après compilation, fermer toute autre application utilisant la caméra. Les
commandes suivantes exigent chacune un dossier de destination vide et créent
uniquement des dossiers patients fictifs, sans modifier les paramètres personnels :

```sh
target/release/iriscope-app --validate-hardware target/validation-auto
target/release/iriscope-app --validate-button target/validation-bouton
```

Le premier scénario prend les photos des deux yeux, réalise deux prises
rapprochées pour vérifier les noms uniques, enregistre et arrête une vidéo, puis
prend une photo anonyme. Le second demande deux prises photo avec le bouton
physique, puis une pression pour démarrer et une pour arrêter la vidéo. Suivre
les messages `BUTTON_*_READY` du terminal. Les essais vérifient les fichiers
lisibles, les numéros de dossiers distincts malgré des noms identiques, l'œil,
la date et l'heure, la finalisation AVI, l'export MP4 avec l'original conservé et
la sauvegarde/restauration des médias avec leurs associations.

Chaque destination contient `captures/`, `exports/`, `backups/`, `restored/` et
`validation-report.txt`. Les résultats et captures d'essai ne sont pas inclus
dans Git. L'export de validation nécessite FFmpeg.

Le [compte rendu Linux du 6 octobre 2026](docs/VALIDATION-LINUX-2026-10-06.md)
confirme les photos et le démarrage/arrêt vidéo au bouton sur l'appareil réel,
avec les dossiers, noms, export et sauvegarde/restauration vérifiés.

Le [rapport d'interface du même jour](docs/VALIDATION-INTERFACE-2026-10-06.md)
décrit les essais de présentation, menus, miniatures, observations et zoom,
ainsi que les vérifications de compilation Windows et macOS depuis Linux.

Pour les tests natifs sous Xvfb, le backend logiciel permet aussi les instantanés
de fenêtres cachées :

```sh
SLINT_BACKEND=winit-software xvfb-run -a cargo test --release --workspace --locked
```

Les contrôles visuels Linux utilisent des captures synthétiques, deux thèmes et
quatre tailles de fenêtre, à 100 %, 125 %, 150 % et 200 %. Après installation de
Pillow, lancer :

```bash
xvfb-run -a -s '-screen 0 4096x2304x24 -nolisten tcp' python3 scripts/check-ui-visuals.py
```

Les 784 images de référence couvrent la barre caméra, les réglages image, les noms
patients longs, les paramètres, le plein écran, les messages de capture longs,
la correction de dossier, la recherche et la comparaison des photos ainsi que la visionneuse vidéo.
Elles incluent la progression d’une sauvegarde, son rappel, l’export MP4 et les
outils photo : notes, références, affichage, choix de comparaison, zoom et loupe,
ainsi que les dossiers de consultation, le calendrier et l’export photo séparé.
Le script rend 1 824 vues, dont les réglages de rotation ouverts, la bibliothèque, les références et la visionneuse photo.
Une modification visuelle volontaire se valide avec `--update`, puis une revue
des PNG ; la CI compare sans réécrire les références. `--compare-only` permet de
comparer les captures déjà générées, sans relancer le rendu.
`--release` utilise les scénarios compilés avec optimisation pour accélérer les
grandes matrices, avec les mêmes comparaisons d’images.
La simulation du changement d’échelle vérifie le rendu logiciel ; les bureaux
natifs Windows/macOS et la caméra réelle nécessitent des essais sur ces systèmes.

### Distribution portable

La distribution retenue pour l’usage personnel est sans signature officielle.
Construire et installer ces paquets ne nécessite ni certificat payant ni abonnement.
La signature reste une option du script, pas une condition de cette installation.

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

Ces archives sont portables : elles ne créent pas de raccourci ni de mise à jour automatique. `python scripts/package-installer.py` produit aussi un paquet Debian avec entrée de menu, un installateur Windows par utilisateur et un DMG macOS avec accès au dossier Applications. La CI prépare les installateurs avec leurs empreintes, en plus des archives portables. Sous Windows, Inno Setup 6 est requis ; sous macOS, `hdiutil` est requis.

La CI produit des paquets Mac Apple Silicon **et Intel**, ainsi que Windows x64.
Le guide [Essayer sur Windows et macOS](docs/ESSAIS-WINDOWS-MACOS.md) indique
quel paquet choisir et comment vérifier la caméra et le bouton sans environnement
de développement. L’absence de FFmpeg est distinguée d’un échec de capture dans
les rapports d’essai matériel.

Les paquets créés sans certificat restent non signés. Pour une distribution publique, le script accepte `--windows-certificate` (empreinte d’un certificat déjà installé), ou `--signing-identity` et `--notary-profile` sous macOS (identité Developer ID et profil de trousseau existants). Il signe et vérifie les paquets Windows ; sous macOS, il signe le bundle, soumet la notarisation et agrafe le ticket avant de créer le DMG. Aucun certificat ni mot de passe n’est inclus dans le dépôt. Ces opérations demandent les certificats officiels et les outils natifs ; elles n’ont pas été exécutées dans l’environnement cloud Linux. Les mises à jour restent manuelles. Le
paquet Linux est construit sur Ubuntu 24.04 et dépend des bibliothèques système
requises par Slint et V4L2 ; sa compatibilité avec d'autres distributions doit
être vérifiée séparément.

Chaque paquet contient le numéro de version et `release-info.json`, avec
l’empreinte du binaire fourni et la disponibilité du moteur MP4. Les scripts
refusent un binaire dont `--version` ne correspond pas au manifeste. Les paramètres
et captures restent hors des fichiers installés ; les identifiants des installateurs
restent constants lors d’une mise à jour. Les instructions sont dans
[docs/RELEASE-0.4.0.md](docs/RELEASE-0.4.0.md).

Pour inclure un FFmpeg autonome construit pour le système cible, les deux scripts
acceptent `--ffmpeg chemin/ffmpeg` et `--ffmpeg-license chemin/COPYING.txt`.
Fournir la licence et les attributions correspondant au binaire ; celui-ci doit
disposer de `libx264`. Le paquet Debian recommande aussi le FFmpeg du système.

### Développement et tests

Pour lancer l'application et juger la latence du live dans les conditions de
distribution :

```text
cargo run --release -p iriscope-app
```

Le profil de développement optimise également les crates de décodage d'image,
mais le profil `release` reste la référence pour les mesures de performance.

Sous Linux, installer FFmpeg et `ffprobe` pour les tests réels d’export, ainsi que
Xvfb pour les interactions graphiques. Les tests des paquets se lancent avec
`python3 -m unittest discover -s tests -p 'test_packaging.py' -v` ; `dpkg-deb` est
nécessaire au contrôle du paquet Debian. Les outils de création d’installateurs
Windows/macOS sont simulés dans ces tests de structure ; leurs installateurs doivent
être construits sur les hôtes natifs.

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

Les mesures reproductibles sont décrites dans [docs/PERFORMANCE.md](docs/PERFORMANCE.md), et les essais avec le DE400 dans [docs/VALIDATION-MATERIELLE.md](docs/VALIDATION-MATERIELLE.md).
