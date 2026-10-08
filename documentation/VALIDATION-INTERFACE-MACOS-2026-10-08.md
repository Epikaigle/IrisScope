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
