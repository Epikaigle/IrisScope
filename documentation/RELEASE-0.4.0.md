# Iriscope 0.4.0

Cette version permet d’adapter l’espace de travail pour la capture, l’examen d’images et la présentation au patient.

- Disposition mémorisée : panneau patient, panneau photo, largeurs, grille ou liste et ouverture des détails techniques.
- Mode présentation : informations personnelles, notes et annotations masquées, commandes de capture et d’arrêt toujours accessibles.
- Image seule : barres photo et panneau retirés pour agrandir la zone de l’iris ; retour des commandes au mouvement, bouton Commandes et sortie par Échap.
- Trois panneaux redimensionnables, limités selon l’espace disponible, avec réglage au clavier et retour à la largeur adaptative.
- Diagnostic caméra repliable, état de connexion visible.
- Miniatures petites, moyennes ou grandes en grille et en liste, avec largeur personnalisable de 80 à 480 pixels logiques.
- Rotation précise de 0 à 359°, zoom immédiat et molette synchronisée dans le direct et les références.
- Calendriers contextuels, bibliothèque compacte et menus alignés qui ne changent plus de sélection à la molette.
- Ajouts et suppressions externes détectés même lorsque le système retarde la mise à jour de la date du dossier, notamment sous Windows.
- Site de présentation de l’application avec modèle 3D du DE400 à l’accueil, captures de l’interface et publication GitHub Pages depuis `main /docs`.
- Bouton physique DE400 validé sous Linux et sur Mac Intel ; vidéo, bouton et huit réglages USB accessibles sur le Mac après installation du composant inclus. Le récepteur Windows reste à confirmer sur l'appareil réel.

Les options sont regroupées dans **Paramètres → Interface et affichage**. **Réinitialiser la disposition** conserve le thème, le stockage, les dossiers, les notes et les images. Aucun réglage supplémentaire ne masque l’aide existante.

## Installer et mettre à jour

Fermer Iriscope, sauvegarder la bibliothèque, puis remplacer l’application. Les anciennes configurations sont compatibles ; les nouvelles options ont des valeurs par défaut. Aucun dossier ni média n’est rouvert automatiquement. Le mode présentation mémorisé rétablit sa vue plein écran pour conserver le masquage des informations personnelles.

Vérifier `VERSION.txt`, `release-info.json`, `iriscope-app --version` et la somme SHA-256 du paquet. Le paquet macOS utilise une signature ad hoc gratuite, sans certificat Developer ID ni notarisation Apple. La version cloud fournit l’archive portable Linux et le paquet Debian. Les installateurs Windows/macOS nécessitent leurs systèmes natifs.

Le mode présentation protège l’affichage dans Iriscope : les fichiers et exports restent complets. Son état est mémorisé jusqu’à sa désactivation. Le [rapport Mac Intel](VALIDATION-MACOS-2026-10-08.md) détaille l'essai du DE400 ; les essais Windows et ceux d'autres Mac restent à effectuer selon [le protocole matériel](VALIDATION-MATERIELLE.md). Le [guide Windows et macOS](ESSAIS-WINDOWS-MACOS.md) explique l’installation et les essais sans environnement de développement.

Les commandes et réglages sont décrits dans [Visionneuse photo](VISIONNEUSE.md).

## Validation disponible

234 tests réussis, deux essais caméra ignorés sans appareil ; 784 comparaisons visuelles sur 1 824 rendus Linux, à 100, 125, 150 et 200 %. La compilation passe avec Clippy strict sous Linux et pour Windows x64 ainsi que macOS x64/ARM64. Dix tests Python couvrent la passerelle et l’empaquetage ; les outils d’installateur Windows/macOS sont simulés dans les tests locaux. Le paquet Debian a été lancé avec une ancienne configuration : panneau mémorisé, fermeture normale et original JPEG conservé. Les notes ne deviennent éditables qu’après lecture complète de leurs observations. Les [rapports d’interface](VALIDATION-INTERFACE-2026-10-06.md) et [matériel Linux](VALIDATION-LINUX-2026-10-06.md) détaillent les limites des essais.

La [passe d'interface du 8 octobre sur Mac](VALIDATION-INTERFACE-MACOS-2026-10-08.md)
fixe les commandes de capture dans la sidebar, réserve la place des barres de
défilement, corrige le menu Miroir et conserve le cadre de focus pour le clavier.
La seconde passe harmonise les thèmes des réglages, menus et visionneuse,
compacte les menus et calendriers, fixe aussi les champs patient, aligne la
grille à gauche et clarifie les dossiers vides. Les commandes photo sont
regroupées dans « Outils » et « Vue ». La couverture atteint 896 références
visuelles sur 1 024 rendus locaux.

Le complément corrige le centrage du bouton Dossier et le défilement jusqu'au
dernier bouton Ouvrir à la taille maximale des miniatures. La mention de
confidentialité passe dans une aide au survol, et le mode de présentation est
nommé explicitement pour le patient. Les scènes ajoutées portent la matrice
à 1 056 rendus et 928 références.
