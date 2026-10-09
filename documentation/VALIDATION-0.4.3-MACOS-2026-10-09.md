# Validation de la 0.4.3 sur macOS Intel — 9 octobre 2026

Mac Intel, macOS 15.7.7, DE400 connecté. Les preuves, captures de test et
sauvegardes de paquets restent dans `target/app-icon-20261009/`, hors Git.

## Icône et paquets

Le symbole reprend l’iris de l’interface, avec les couleurs du thème clair :
fond blanc fixe et iris doré. Le paquet Mac déclare `CFBundleIconFile` et inclut
l’ICNS haute résolution. Le paquet Debian installe le même SVG, avec
`Icon=iriscope`, `StartupWMClass=iriscope` et l’identifiant X11/Wayland correspondant.

Les images PNG, la transparence, les représentations ICNS et les tailles ICO
sont contrôlées. L’icône native du paquet est relue par `NSWorkspace` et son
affichage blanc dans le Dock est confirmé à l’écran.

La compilation optimisée application/updater prend 7 min 05 s. Le passage final
au fond blanc remplace les ressources du paquet Mac sans refaire cette compilation.
Sur Mac, Winit ignore l’icône de fenêtre ; le Dock utilise celle du bundle.
Les exécutables de l’application, de l’updater et des composants USB sont conservés.
Linux est construit à partir des ressources finales par le workflow existant.

## Contrôles logiciels et mises à jour

- Dix tests Rust de l’updater passent en release ; Clippy strict ciblé passe.
- Dix-neuf tests Python passent ; la construction Debian native est ignorée sur Mac.
- Le formatage Rust, les différences Git et les contrôles du site passent.
- Les notes de release transportent les octets exacts du manifeste et de la signature
  dans un commentaire non affiché. Les tests vérifient la signature Ed25519,
  le rejet des métadonnées malformées et la publication de deux installateurs seulement.

Le `.dmg` final, signé ad hoc gratuitement, remplace réellement une 0.4.2 à travers
l’installateur de production et des métadonnées locales signées par la clé de mise
à jour. Il est monté en lecture seule, le bundle est copié, le disque éjecté,
la signature vérifiée et la nouvelle application redémarrée. L’installateur reçoit
l’accusé de démarrage et supprime son cache.

Le lanceur de test terminal est maintenu actif jusqu’au résultat : le premier essai
avait été interrompu par la fin de sa session terminal, avant le remplacement.
Le test final démarre depuis l’ancien `Contents/MacOS`, pour vérifier aussi le
remplacement d’un dossier de travail situé dans l’application.

Les 55 fichiers installés et leurs permissions correspondent au DMG testé.
Les empreintes des réglages et des médias existants restent inchangées.
Le nouveau paquet USB est approuvé une fois ; le service confirme ensuite un accès
normal UID 501 sans nouvelle authentification.

Les 0.4.1/0.4.2 nécessitent une installation manuelle initiale vers la 0.4.3 ;
le nouveau format utilise ensuite les mêmes `.dmg` et `.deb` pour les mises à jour.

## Essai DE400

Le vrai flux caméra produit quatre PNG de 1280 × 1024 et une vidéo AVI de
32 images à 8 fps. Les deux yeux, les dossiers homonymes, la capture sans dossier
et la sauvegarde/restauration de cinq médias sont vérifiés. Les réglages caméra
et le récepteur USB du bouton sont disponibles.

Les déclenchements de ce complément sont logiciels, via le gestionnaire du bouton.
Le lecteur et le service USB sont identiques aux versions déjà testées physiquement.
Le changement final de fond d’icône ne modifie aucun de ces exécutables.
Le flux caméra reste visible après l’installation du paquet blanc final.

L’export MP4 avec FFmpeg optionnel et l’authentification/installation Debian réelle
sur Linux restent hors de cet essai. Windows et Apple Silicon ne sont pas publiés.

## Publication vérifiée

La [release 0.4.3](https://github.com/Epikaigle/iriscope-app/releases/tag/v0.4.3)
est stable et contient exactement deux installateurs : `IrisScope-0.4.3-macOS-Intel.dmg`
et `IrisScope-0.4.3-amd64.deb`. Le
[workflow existant](https://github.com/Epikaigle/iriscope-app/actions/runs/37938958584)
réussit pour les deux systèmes. Le tag pointe sur le commit testé de `main`,
`01fffc02469eed8cebb7b787e7f52a0e80ae1075`. Le DMG publié correspond exactement
au paquet installé et contrôlé sur le Mac.

Les métadonnées lues dans les notes publiques passent la vérification Ed25519.
Les tailles et empreintes des deux installateurs correspondent au manifeste signé.
La sonde liée à la bibliothèque de production télécharge et vérifie le DMG public
complet en simulant une version installée plus ancienne. L’application installée
confirme ensuite qu’elle est à jour.

Le Debian réel téléchargé contient les exécutables ELF x86_64, les permissions,
les métadonnées, la nouvelle icône et les notes attendues. L’empreinte du programme
correspond à `release-info.json`, avec les dépendances curl/pkexec, des fichiers
appartenant à root et aucune donnée utilisateur ni hook de suppression.
Ces contrôles ne remplacent pas une installation Polkit/dpkg sur Linux.

Après ces vérifications, les releases 0.4.1 et 0.4.2 sont supprimées à la demande
de l’utilisateur ; seule la 0.4.3 reste dans la liste des releases. Les deux liens
automatiques GitHub vers les archives du code source s’ajoutent aux installateurs.
L’application optimisée 0.4.3 reste ouverte avec son icône blanche et le DE400 connecté.
