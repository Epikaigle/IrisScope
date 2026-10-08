# Interface sur petit écran — Mac Intel, 8 octobre 2026

Cette passe corrige l'interface après la validation du [service USB et du bouton
DE400](VALIDATION-MACOS-2026-10-08.md). Le poste est un MacBookPro12,1 sous
macOS 15.7.7. Les médias et dossiers d'essai sont séparés de la bibliothèque
personnelle.

## Disposition et interactions

- Les choix de l'œil, du mode photo/vidéo et la capture restent visibles en bas
  de la sidebar. Seuls l'identité, la recherche de dossiers et le dernier média
  défilent. La présence d'une miniature ne déplace plus ces commandes.
- Une marge stable protège les champs et actions à droite des zones qui
  défilent : formulaire, réglages caméra, bibliothèque et outils photo.
- Les deux choix de l'œil et du mode ont la même largeur. Le bouton de fin de
  session conserve son action et son libellé accessible avec un texte plus court.
- Le clic à la souris n'ajoute plus le cadre de focus intérieur. La navigation
  au clavier conserve ce repère, y compris après retour d'un menu.
- Les boutons sélectionnés gardent les tons chauds de leur sélection au survol
  et pendant l'appui.
- Le menu Miroir s'ouvre au-dessus de la barre entière, y compris quand elle
  occupe deux lignes. Les menus déroulants restent dans les limites de la fenêtre.
- Les widgets utilisent explicitement le style Fluent sur tous les bureaux,
  pour conserver les mêmes dimensions entre macOS, Linux et Windows.

## Vérification locale

Les essais Slint de contrôles passent sur le Mac avec le renderer logiciel :
activation unique par Espace/Entrée, répétition, annulation lors d'une perte de
focus ou désactivation, pas de cadre au clic, retour de focus après fermeture
du menu Miroir, sélection des deux miroirs, pas des curseurs, menus, rotation,
calendriers et zoom à la molette. Les fenêtres d'essai ont été visualisées
avec le viewer Slint natif avant la compilation du GUI.

La matrice synthétique couvre 61 scénarios, deux fenêtres (800 × 600 et
1360 × 860), deux thèmes et quatre échelles (100, 125, 150 et 200 %), soit
976 captures. Elle ajoute les cas après capture, sidebar de 260 pixels, menu
Miroir et sélection à la souris. La comparaison utilise 848 références revues.
Les fixtures ne contiennent aucune image caméra ni donnée patient. Le mode
hors écran évite les contraintes de thread et de taille du moniteur macOS.

Le GUI et l'outil de rendus sont compilés en mode release optimisé. Les
bibliothèques et composants USB déjà validés sont réutilisés. Les 13 tests
Python sont exécutés : 12 passent et le test Debian est ignoré sur Mac faute
de `dpkg-deb`. Le formatage Rust et le contrôle des espaces passent.
Clippy strict passe en release pour le GUI, l'outil de rendus et les deux tests
Slint concernés. Le workspace complet n'est pas recompilé pour cette passe.

## Essai du paquet installé

Le ZIP et le DMG sont signés ad hoc gratuitement et leurs signatures sont
vérifiées. Leurs 49 fichiers sont identiques ; les sommes SHA-256 des deux
archives passent. L'application installée provient du DMG. L'essai ci-dessous
a lieu après la génération des paquets ; cette confirmation est ajoutée au
rapport du dépôt.

La commande `--validate-hardware` passe avec le DE400 connecté : quatre PNG
de 1280 × 1024, AVI de 32 images intégralement décodables à 8 fps, attribution
des yeux et dossiers, noms distincts pour les homonymes, sauvegarde/restauration
de cinq médias identiques. Le test utilise la commande de capture en production
et des signaux logiciels ; aucun nouvel appui physique n'est demandé pour cette
correction d'interface. MP4 n'est pas testé sans FFmpeg optionnel.

Le lecteur et le service USB sont identiques octet pour octet à la version
validée physiquement auparavant. La mise à jour de l'autorisation de cette
application passe ; les ouvertures et la validation suivante s'exécutent sous
le compte utilisateur, sans demande de mot de passe. À la fermeture du test,
le lecteur s'arrête et ses dossiers temporaires disparaissent. L'application
ordinaire est ensuite relancée avec la nouvelle interface.

```text
SHA-256 IrisScopeGui installé, après signature :
9b1c67bb0b9ad1253afa9426fefd0e8be84e9035cd08fc614b1d04ad5d2995d8
```

Les sources, rendus et journaux de cette passe restent sous
`target/ui-repair-20261008/`, hors Git. Les contrôles logiciels ne prouvent pas
le comportement matériel sur d'autres ordinateurs. Les composants USB n'ont
pas été modifiés par cette passe d'interface ; l'essai physique précédent est
documenté séparément. Aucun nouvel essai physique Windows ou Apple Silicon
n'est présenté comme réalisé.

## Seconde passe — thèmes, menus et organisation

Cette passe remplace la disposition initiale décrite plus haut : l'identité,
le numéro de dossier et les commandes œil/mode/capture restent désormais tous
fixes. Seuls les résultats de recherche et la dernière capture ont leur propre
zone de défilement. Le menu Dossier conserve l'accès à l'historique, à la
correction d'identité et au changement de personne.

Les réglages caméra, les listes de choix, les calendriers, les dossiers et la
visionneuse suivent la palette claire ou sombre. Les fonds des fenêtres et
panneaux sont opaques ; les vrais canevas d'image restent sombres avec des
libellés adaptés. Les références vides utilisent la palette du thème.
Les menus se dimensionnent selon les libellés, avec des lignes de 32 pixels et
des marges réduites. Le calendrier utilise une icône et garde son nom accessible.

La grille calcule explicitement les positions des cartes pour garder la
première à gauche, y compris après redimensionnement et avec deux médias.
L'état de connexion occupe le bord droit de l'en-tête. La vue Dossiers / Séances
vide explique la recherche ; les notes et prises ne se présentent qu'après
sélection d'une séance. La sauvegarde des notes et ses états d'erreur sont
conservés.

La barre photo passe de trois à deux lignes. « Outils » regroupe annotations,
comparaison, références, réglages et rétention ; « Vue » regroupe image seule,
taille réelle, loupe, plein écran et présentation. Le zoom, l'ajustement,
la navigation, l'export et la fermeture restent directs. Les commandes gardent
les restrictions de chargement et de présentation. Les outils d'annotation
utilisent un sélecteur et les dates de comparaison peuvent se replier.

### Contrôles logiciels de cette passe

- Deux tests natifs de contrôles réussis : boutons, menus, choix unique,
  commandes désactivées, clavier, focus, calendriers, rotation et molette.
- 64 scénarios × deux tailles × deux thèmes × quatre échelles : **1 024 rendus**.
  Les **896 références** couvrent notamment les deux nouveaux menus photo et
  la bibliothèque peu remplie. Les rendus concernés sont revus visuellement.
- Des assertions vérifient l'action des menus, la palette de la visionneuse,
  l'alignement à gauche et l'identité des pixels du formulaire avant/après
  défilement de la dernière capture. Aucun média ni dossier personnel utilisé.
- Clippy strict en release réussit pour le GUI, l'outil de rendus et les deux
  tests concernés. Formatage Rust et contrôle des espaces réussis.
- Tests Python : **12 réussis, un ignoré** faute de `dpkg-deb` sur macOS.

Le GUI est compilé en release optimisée avec les fixtures regroupées. Après
adaptation de deux essais, seules les fixtures ont été recompilées ; aucune
modification du GUI n'a suivi la compilation finale. Les composants USB natifs
déjà validés sont réutilisés. Les paquets utilisent la signature ad hoc gratuite.
Les éléments logiciels ci-dessus sont inclus dans les paquets ; la confirmation
matérielle après installation sera ajoutée au rapport du dépôt.

```text
SHA-256 du GUI release avant signature :
80de945fe1480bcd038f10e165dcde235098ebde954d5309eab5d4b3e116c1a0
```

Les preuves de cette passe sont sous `target/ui-consistency-20261008/`, hors Git.
Les essais logiciels sur Mac ne remplacent pas des essais matériels Windows
ou Apple Silicon.

### Confirmation après installation du second paquet

Les 896 comparaisons de référence passent. Les sommes SHA-256 du ZIP et du DMG
sont vérifiées, les signatures ad hoc sont valides et leurs **49 fichiers sont
identiques**. Le lecteur et le service USB signés sont identiques octet pour
octet à ceux du paquet précédent. L'application locale est remplacée depuis
le DMG ; une copie de l'ancienne application est conservée dans le cache d'essai.

Après mise à jour de l'autorisation du paquet, `--usb-service-status` confirme
le service sous le compte utilisateur (UID 501), sans dialogue administrateur.
`--validate-hardware` passe avec la caméra DE400 : **quatre PNG 1280 × 1024**, une
**vidéo AVI de 32 images à 8 fps**, attribution des yeux et des deux dossiers
homonymes, prise sans dossier et sauvegarde/restauration de **cinq médias
identiques**. Les neuf entrées de réglage sont présentes, dont l'anti-scintillement
et la balance des blancs automatique. Le récepteur USB du bouton est actif.

Ce test utilise les commandes de production avec déclenchements logiciels,
sans nouvel appui physique. L'export MP4 reste non testé sans FFmpeg optionnel.
À la sortie, le lecteur USB s'arrête et aucun dossier temporaire de session ne
subsiste. L'application ordinaire est relancée sous le compte utilisateur.
Cette confirmation postérieure à la génération des paquets figure dans le
rapport du dépôt ; elle ne modifie pas le contenu déjà signé du ZIP et du DMG.

```text
SHA-256 IrisScopeGui installé après signature :
492120bf7b3d0f1e0fa787e9a7afe1a2d89b551605f804b99a1bba7764b87a1d
USB revision :
fcbd7be3e06476012380b7c6f05fcef20715f36075d1dc0b9beb1fef95306f1c
ZIP SHA-256 :
fc1d124f62340792bad330013ecae758619bc796d44da911a457673da621f291
DMG SHA-256 :
fbffd0e9e1271d88dee8f4d0ae92d7f67e6807420303f4399023a6d941cad261
```
