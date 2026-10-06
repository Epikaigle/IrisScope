# Suivi des corrections et de la structure

Suivi du travail des 30 septembre, 1er et 2 octobre 2026. Ce document distingue les corrections de code et leur validation sur du matériel réel.

| # | Sujet | Correction et limite |
|---|---|---|
| 1 | Index bibliothèque corrompu | Sauvegarde du dernier index, restauration et conservation de l'index abîmé. Refus de recréer silencieusement les dossiers si aucune copie valide n'existe. |
| 2 | Vidéos temporaires après interruption | Détection des `.part`, récupération des images complètes et publication indexée. La vidéo récupérée reste sans dossier jusqu'à confirmation de l'opérateur. Les très anciens fichiers dépourvus d'en-tête exploitable ne sont pas réparables automatiquement. |
| 3 | Patients homonymes | Dossiers distincts avec numéro stable, liste pendant la saisie, sélection explicite, recherche directe par numéro. Aucun autre renseignement personnel demandé. |
| 4 | Formats caméra inconnus | Les modes indécodables ne sont plus tentés comme s'ils étaient pris en charge. Erreurs explicites à la capture. |
| 5 | Contraste du thème clair | Couleurs et états revus dans le thème partagé. |
| 6 | Arrêt Windows bloqué dans ReadSample | Lecture et commandes isolées sur le worker ; requêtes et fermeture bornées. Essai de débranchement physique sous Windows encore nécessaire. |
| 7 | Média et métadonnées publiés séparément | Journal durable avant écriture du média final, reprise du lien avec le dossier sélectionné, staging conservé jusqu'au commit. Fichiers modifiés refusés lors de la reprise. |
| 8 | Systèmes de fichiers sans liens physiques | Repli par copie en flux, destination exclusive et collisions conservées. Aucun chargement d'une vidéo entière en mémoire. |
| 9 | Échec affiché malgré un média déjà enregistré | Résultat distinct pour le média publié et l'association en attente. Avertissement explicite et reprise du journal. |
| 10 | Métadonnées anciennes ou périmées | Empreinte SHA256 et version du fichier, exclusion des liens symboliques, capture remplacée désassociée. Une copie identique conserve son dossier. Les versions affichées sont vérifiées lors de l'ouverture et du rattachement, ainsi que pendant la lecture vidéo. Les noms anciens ne deviennent jamais une identité de dossier. |
| 11 | Découpage ambigu des noms de fichiers | L'identité provient de l'index ; les captures anciennes restent sans numéro de dossier. |
| 12 | Jetons de nommage réinterprétés | Remplacement en une passe, sans réinterpréter le contenu des noms. |
| 13 | Suffixes de collision trop longs | Longueur finale bornée en octets, y compris extension et suffixe. Noms portables. |
| 14 | MJPEG invalide dans une vidéo | Validation JPEG avant écriture, comptage des images rejetées. |
| 15 | Timing vidéo irrégulier | Cadence AVI nominale, maintien de la dernière image pendant les pertes, y compris les pertes de queue. Sauts anormaux et écritures de rattrapage bornés. |
| 16 | AVI externes reconnus à tort | Validation structurelle avant lecture intégrée ; ouverture des autres vidéos dans le lecteur système. |
| 17 | Tri perturbé par la copie des fichiers | Date de capture de l'index prioritaire ; mtime uniquement en repli. |
| 18 | Cadences mal déduites | Cadences rationnelles conservées, sélection des modes disponibles et lecture fondée sur le FPS AVI. |
| 19 | Réinitialisation Linux incomplète | Réinitialisation déterministe des contrôles pris en charge et remontée des erreurs. |
| 20 | Contrôles Windows/macOS et bouton physique | Windows : interfaces IAM si disponibles, repli IKsControl standard, valeurs/plages validées et commandes bornées. macOS : modes d'exposition et balance des blancs fournis par AVFoundation. Bouton physique : aucun événement clavier/V4L2 publié sur l'exemplaire branché ; écoute des métadonnées UVCH fonctionnelle, mais pression du bouton encore à vérifier. |
| 21 | MJPEG natif macOS | Dictionnaire de sortie AVFoundation vide en mode MJPEG natif, vérification du codec reçu et taille de frame plafonnée. Validation matérielle sur macOS nécessaire. |
| 22 | V4L2 multi-plane | Stream MMAP dédié avec mapping par plane, MJPEG/YUYV/BGRA/NV12/NV12M progressifs et linéaires ; retrait du padding et validation des offsets. Plafonds de 64 Mio/frame et 256 Mio de mappings. Le DE400 mono-plane garde son chemin existant. Aucun matériel multi-plane disponible pour vérifier les ioctls réels. |
| 23 | Bibliothèque relue entièrement à chaque page | Cache du scan trié et des sélections de filtre, page de 100 éléments, vérification des fichiers visibles. Invalidation sur changement du répertoire/index, actualisation explicite et rescan après 30 secondes lors d'une nouvelle requête. |
| 24 | Pics mémoire des images de référence | Fichiers plafonnés à 32 Mio, images à 16 mégapixels, décodages sérialisés et cache de miniatures de 32 Mio. |
| 25 | Sauvegarde synchrone des réglages | Worker avec regroupement de 500 ms, copie de l'état avant écriture disque et vidage à la fermeture. |
| 26 | Workers abandonnés ou fermeture lente | Mailboxes fermées explicitement, attente bornée des threads et annulation des résultats obsolètes. |
| 27 | Erreurs masquées | Retours visibles pour stockage, index, lecture, conversion, réglages et reprise. Succès/avertissement distingués. |
| 28 | Clavier et focus | Raccourcis bloqués pendant la saisie, navigation clavier, restitution du focus après fermeture de la visionneuse. |
| 29 | Plages de contrôle et fenêtres étroites | Pas et limites vérifiés sans débordement, grands entiers manipulés sans slider flottant imprécis, affichage et diagnostics adaptés. |
| 30 | Planches et distribution | Imports d'images de référence fonctionnels ; aucune planche tierce embarquée sans licence vérifiée. Paquets portables Linux/Windows/macOS, autorisation caméra macOS, licences de polices et empreintes SHA256. |

## Corrections de structure et de performance

Ces changements prolongent les 30 corrections fonctionnelles ci-dessus.

| Sujet de l'audit | Changement |
|---|---|
| Assemblage de l'application trop volumineux | `main.rs` choisit le mode, `gui.rs` assemble la fenêtre et les contrôleurs par fonction. |
| Dépendances implicites entre modules | Imports de production explicites ; le binaire ne réexporte plus les dépendances de tous les modules. |
| Bibliothèque monolithique | Patients, index/migration, scan, présentation et transactions séparés ; validation de fichiers partagée avec le lecteur AVI. |
| Workers et queues dispersés | Pipelines photo, vidéo, bibliothèque et références dans leurs modules ; état et cycle de vie possédés par `AppRuntime`. |
| Crate et dépendances inutilisés | `iriscope-renderer` retiré ; imagerie indépendante du cœur. Six crates, aucun cycle. |
| Interface générée plusieurs fois | Une inclusion Slint dans `ui.rs`, partagée par l'application et les tests ; tests des vrais contrôleurs avec caméra simulée. |
| Annulation et fermeture tardives | Annulation entre entrées de scan et blocs SHA ; budget commun pour attendre les workers. Les captures admises sont finalisées avant fermeture de la fenêtre. |
| Opérations disque dans les callbacks | Opérations lentes confiées à un exécuteur borné, avec remontée de leurs erreurs. |
| SHA de toute la bibliothèque à chaque scan | Scan initial des métadonnées triées, SHA ciblé sur la page visible et vérification du dossier avant affichage. Cache d'empreintes borné avec éviction amortie. |
| Verrou d'index pendant la copie vidéo | Préparation, copie et validation du média hors verrou d'index ; verrou de publication distinct et relecture fraîche de l'index au commit. |
| Un thread par changement de référence | Un worker possédé par le runtime, dernières demandes regroupées par type, annulation avant/après verrou et pendant la lecture. |
| Queues bornées seulement en nombre | Budgets de 64 Mio pour les photos et la vidéo ; formats caméra limités avant les copies natives. |
| Copie complète avant miniature | Redimensionnement à partir du buffer RGB emprunté, avec allocation du résultat seulement. |
| Énumération de menus V4L2 non bornée | Maximum de 256 tentatives et 128 choix, plages invalides signalées. |
| Anciennes API moins sûres | Décodeur d'image public soumis aux mêmes limites ; publication legacy par lien complet uniquement, sans copie partielle au nom final. Le chemin indexé de l'application conserve sa copie en flux. |
| Empaquetage et vérifications redondantes | Exécutable et SHA du paquet traités en flux ; Clippy compile/analyse tous les targets sans refaire un `cargo check` identique. |

Les limites communes sont de 8192 pixels par côté, 16 mégapixels, 32 Mio de
MJPEG et 64 Mio de frame brute. Elles bornent les buffers, sans constituer un
plafond global de RAM de l'application. Les captures d'au plus 1 Mio sont
revérifiées intégralement pendant le commit, sans attente de stabilité. Pour les
médias plus grands, la validation attend la stabilité du fichier hors verrou
d'index ; sur un système de fichiers faible
ou encore instable, une relecture SHA sous verrou reste nécessaire pour protéger
l'association avec le dossier. Les journaux et les vidéos `.part` préservent les
publications interrompues. Une fermeture forcée par le système peut perdre une
demande photo encore uniquement en mémoire.

## Vérifications de la première intégration

Vérifications réalisées le 30 septembre 2026, après intégration :

- `cargo test --workspace --locked --offline -j1` : **155 tests réussis**, aucun échec. Cela comprend 49 tests de l'application, 2 tests d'interface, 71 du cœur, 17 du backend Linux, 3 de capacités macOS, 12 d'imagerie et 1 du renderer. Les 2 tests matériels sont ignorés dans cette commande.
- `cargo test -p iriscope-camera-linux --locked --offline -j1 -- --ignored --nocapture --test-threads=1` : **2 essais DE400 réussis**. Modes et contrôles lisibles ; réception des images et élimination des frames anciennes confirmées (séquence 1 → 22 après 1,08 s lors du test de faible latence).
- `cargo clippy --workspace --all-targets --locked --offline -j1 -- -D warnings` : réussi sur Linux.
- Même contrôle strict avec `--target x86_64-pc-windows-msvc`, `x86_64-apple-darwin` et `aarch64-apple-darwin` : réussi pour les trois cibles, y compris la compilation des tests. Ces tests croisés ne sont pas exécutés sur Linux.
- `cargo fmt --all --check`, `git diff --check`, compilation Python des deux scripts et auto-test du diagnostic du bouton : réussis.
- `cargo build --release -p iriscope-app --locked --offline -j1` : exécutable Linux optimisé construit. Son diagnostic réel a ouvert le DE400 en MJPEG 1280 × 1024 et décodé les 3 images demandées avant fermeture du flux.

Toutes les compilations locales utilisent un seul job pour limiter la RAM. Les essais matériels concernent le DE400 branché sous Linux ; une compilation croisée ne remplace pas les essais de caméra sous Windows ou macOS. Le bouton physique nécessite encore des pressions corrélées à l'écoute des métadonnées ; le flux multi-plane nécessite un appareil correspondant. Le diagnostic du bouton lancé précédemment sans pression a reçu des métadonnées valides, sans marqueur STILL_IMAGE, ce qui ne suffit pas à conclure sur le bouton.

## Vérifications après réorganisation

Vérifications réalisées le 1er octobre 2026, sur les sources réorganisées :

- `xvfb-run -a cargo test --workspace --locked --offline -j1` : **174 tests réussis**, aucun échec. Répartition : 56 tests applicatifs, 3 tests d'interface et des vrais contrôleurs, 78 du cœur, 19 du backend Linux, 3 de capacités macOS et 15 d'imagerie. Les 2 essais matériels sont ignorés dans cette commande.
- Les nouveaux tests couvrent notamment 300 associations devenues invalides avant une capture valide, l'annulation des lectures et des empreintes, une migration d'index v1 interrompue puis reprise, les budgets des queues et le vrai événement de fermeture avec photo et vidéo en attente.
- `cargo clippy --workspace --all-targets --locked --offline -j1 -- -D warnings` : réussi sur Linux et avec les trois cibles `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Les tests sont compilés sur ces cibles ; leur exécution native Windows/macOS reste à réaliser.
- `cargo test -p iriscope-camera-linux --locked --offline -j1 -- --ignored --nocapture --test-threads=1` : **2 essais DE400 réussis**. Faible latence : séquence 1 vers séquence 20 après 1,064 seconde. Modes et contrôles accessibles confirmés.
- `cargo fmt --all --check`, `git diff --check`, vérification de syntaxe Bash/Python et auto-test du diagnostic du bouton : réussis.
- `cargo build --release -p iriscope-app --locked --offline -j1` : binaire Linux optimisé construit après réorganisation. `target/release/iriscope-app --diagnose` a ouvert le DE400 en MJPEG 1280 × 1024 à 8 FPS annoncés, reçu et décodé trois images de 168 à 169 Ko, puis fermé le flux sans erreur.
- Archive Linux créée avec `scripts/package-release.py` : empreinte SHA-256 vérifiée, exécutable archivé identique au binaire construit, documentation et licences incluses. L'exécutable conserve le mode `0755` et les documents le mode `0644`.

La caméra et le décodage sont arrêtés dès la demande de fermeture. La fenêtre
attend les photos, la vidéo et les mutations de dossiers déjà acceptées ; elle
reste réactive et signale une sauvegarde encore en cours après cinq secondes.
Le budget commun de cinq secondes s'applique ensuite aux autres workers. Les
opérations OS bloquantes, la reprise d'un journal et les commits atomiques ne
sont pas interrompus au milieu de leur protocole. Tous les builds locaux sont
séquentiels, avec un seul job Cargo.

## Refonte de l'interface du 2 octobre 2026

### Résultat de la revue visuelle

Les écrans Caméra, Bibliothèque, Paramètres et leurs overlays utilisent une
hiérarchie commune : une action principale de capture, des sélections explicites
et des outils secondaires plus discrets. L'image reste le centre de l'écran.
Les panneaux correspondent à des fonctions identifiées, sans cartes décoratives
ni effets de verre. Aucun chevauchement bloquant n'a été trouvé dans la revue
finale à 800 × 600 et 1360 × 860 sous Linux.

La refonte conserve les thèmes clair, sombre et système. Les overlays disposent
de leurs propres couleurs lisibles, indépendamment du thème choisi. Les tailles
et espacements communs sont regroupés dans `ui/theme.slint`.

### Défauts identifiés et corrigés

| Sévérité | Catégorie et emplacement | Effet et correction |
|---|---|---|
| Élevée | Clavier — `ui/controls.slint` | Une touche maintenue pouvait répéter une activation. Les boutons déclenchent une seule action et annulent l'appui lors de la perte du focus. |
| Élevée | Capture — panneau patient et contrôleurs | Une capture pouvait démarrer pendant une ouverture de dossier. L'interface et les callbacks bloquent le démarrage, en conservant l'arrêt d'une vidéo déjà en cours. |
| Moyenne | Disposition — `ui/camera-preview.slint` | Le centrage implicite de l'image produisait une bande vide et un recouvrement par les outils. L'image est ancrée en haut et réserve la hauteur de la barre permanente. |
| Moyenne | Thème — overlays et composants communs | Du texte dépendant du thème clair devenait sombre sur les fonds noirs. Les couleurs des overlays sont partagées et indépendantes du thème. |
| Moyenne | Focus — `ui/main.slint` | Fermer un panneau pouvait laisser les raccourcis sans focus. Celui-ci revient au scope principal après fermeture. |
| Moyenne | Zoom — aperçu, visionneuse et références | Un déplacement pouvait sortir du cadre après redimensionnement. Le pan est borné et recalculé ; les flèches déplacent l'image agrandie au clavier. |
| Moyenne | Sauvegarde — paramètres et contrôleur | Resélectionner une valeur après une erreur disque ne relançait pas toujours la sauvegarde. Le même dossier, thème ou niveau de qualité peut être réessayé ; une qualité inchangée ne redémarre pas le flux. |
| Moyenne | Capture — `capture_controller.rs` | Une file pleine pour le son facultatif pouvait annoncer l'échec d'une photo déjà acceptée. L'absence de son ne transforme plus cette capture en échec. |
| Moyenne | Clavier — `ui/library.slint` | Les cartes n'étaient sélectionnables qu'à la souris. Elles ont un libellé accessible, un focus visible et une activation par Espace ou Entrée. |
| Moyenne | Rattachement — `ui/library.slint` | La confirmation n'était liée qu'au patient. Elle suit maintenant le patient, le chemin et la version de la capture, et se ferme si la sélection change. |
| Moyenne | Retours — `ui/settings.slint` | Les messages pouvaient disparaître pendant le défilement. Ils restent hors du formulaire et reviennent à la ligne. |
| Moyenne | Images — `ui/library.slint` | Le recadrage des miniatures pouvait couper le bord de l'iris. L'image complète est conservée avec `contain`. |
| Moyenne | AZERTY — `ui/main.slint` | Les raccourcis numériques refusaient Maj, nécessaire pour produire les chiffres. Ctrl+Maj+1 à 5 est accepté avec les mêmes gardes de saisie et d'overlay. |
| Faible | Hiérarchie — navigation, bibliothèque et paramètres | Les boutons de sélection utilisaient le même traitement que l'action principale. Les états sélectionnés sont distincts et les actions secondaires sont plus discrètes. |
| Faible | Alignement — liste de bibliothèque | Miniatures, badges et boutons avaient des alignements verticaux différents. Ils sont centrés dans les lignes de 92 pixels. |

Les points conservés sont la capture toujours accessible en bas du panneau
patient, les formulaires et réglages défilants, les noms longs tronqués sans
élargir la fenêtre, les états vides avec une action de récupération, et la pause
du décodage du direct lorsqu'une autre page ou un overlay le masque.

### Validation finale

- `xvfb-run -a cargo test --workspace --locked --offline -j1` : **176 tests réussis**, aucun échec ; les deux essais matériels sont séparés.
- Les tests de l'interface couvrent les vrais clics, l'appui clavier maintenu, la perte de focus, les pas natifs des contrôles caméra, les raccourcis AZERTY, les gardes patient, le zoom après redimensionnement et la sélection clavier des cartes. Les tests des callbacks vérifient la reprise du décodage, les sauvegardes réessayées et une capture acceptée malgré la saturation de la file du son.
- Clippy strict sur tous les targets, avec `--locked --offline -j1 -- -D warnings` : réussi en natif Linux et sur `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`.
- Les deux essais matériels DE400 passent à nouveau : modes et contrôles lisibles ; séquence 1 vers 18 après 1,044 seconde pour le test de faible latence.
- `cargo fmt --all --check` et `git diff --check` : réussis. Le binaire Linux optimisé est construit avec `cargo build --release -p iriscope-app --locked --offline -j1` ; son diagnostic ouvre le DE400 en MJPEG 1280 × 1024 et décode trois images avant fermeture du flux. L'archive Linux est reconstruite avec `scripts/package-release.py`.
- Essais de l'application avec le DE400 sous Linux : direct, nouvelles prises photo et vidéo, finalisation et présence dans l'index ; création d'un dossier fictif et photo associée à son numéro stable.
- Sauvegarde réellement empêchée dans le profil de test, puis stockage restauré : le bouton du dossier inchangé relance l'écriture et conserve le nouveau thème. Le profil utilisateur n'est pas utilisé pour cet essai.
- Revue visuelle des pages et overlays à 800 × 600 et 1360 × 860, en thèmes clair et sombre ; grille/liste, lecteur vidéo, zoom, timeline, référence chargée et déplacement à la souris et au clavier. Les imports utilisent des images de test, sans ajouter de planche tierce à la distribution.

Les essais natifs de caméra Windows/macOS, le bouton physique et le matériel
multi-plane restent les validations prioritaires sur les systèmes et appareils
correspondants. La compilation croisée vérifie les API et les tests compilés,
sans les exécuter sur ces systèmes. La revue Linux ne constitue pas un audit
complet avec lecteur d'écran ou facteur d'échelle élevé.

## Corrections et optimisations du 5 octobre 2026

La séparation de la barre caméra est ancrée au-dessus des boutons, et le bouton
« Choisir » des résultats patients est centré dans sa ligne. Les menus caméra
permettent une sélection directe, en conservant les valeurs natives, même
lorsqu'elles sont espacées ou dépassent la précision des nombres flottants.
Les textes longs des boutons, patients et captures disposent d'infobulles
passives : leur apparition ne prend ni le focus clavier ni le premier clic.

Les paramètres proposent « Parcourir… » pour le dossier des captures et les
deux images de référence. Le sélecteur système fonctionne dans un worker et
réutilise les callbacks existants pour valider et sauvegarder les chemins.
Une annulation conserve les paramètres ; la saisie manuelle reste disponible.
Sous Linux, un portail de bureau ou `zenity` est nécessaire.

La bibliothèque présente les métadonnées avant de décoder les miniatures des
éléments visibles. Le défilement actualise la plage prioritaire ; les changements
de page et de dossier invalident les résultats en attente. La version du fichier
est contrôlée avant et après le décodage, et le cache conserve son budget de
32 Mio. La recherche patient réutilise un index normalisé jusqu'au changement
du fichier principal ou de sa sauvegarde, avec un maximum de 25 propositions.
Les buffers Slint du direct, des photos et des vidéos sont préparés dans les
workers avant leur présentation par le thread d'interface.

Validation dans l'environnement cloud Linux, sans caméra connectée :

- **186 tests réussis**, aucun échec ; les deux essais matériels sont ignorés.
- Clippy strict sur tous les targets du workspace (bibliothèques, application, tests et exemple visuel), en natif Linux et pour Windows x86_64, macOS x86_64 et macOS ARM64 : réussi. Ces contrôles croisés n'exécutent pas les bureaux natifs.
- **608 vues synthétiques** : 19 scénarios, quatre tailles, deux thèmes et quatre facteurs d'échelle (100 %, 125 %, 150 %, 200 %). Les dimensions physiques sont vérifiées ; **64 comparaisons d'images réussissent** sur les barres caméra, menus, noms patients et paramètres. La CI Linux compare les références et archive les captures.
- Application réelle sous Xvfb avec **140 captures fictives** : chargement des miniatures visibles en grille et en liste, défilement et fermeture propre vérifiés.
- Retour de sélection simulé via le repli `zenity` : sauvegarde du dossier et des deux références avec espaces et accents, annulation sans modification et fermeture propre vérifiées. Cet essai contrôle les callbacks, sans valider l'affichage du sélecteur natif.
- Formatage Rust et `git diff --check` : réussis.

Les tests d'échelle injectent l'événement du backend et contrôlent le rendu
logiciel. Les essais natifs de caméra et de sélecteur Windows/macOS restent à
réaliser. Le bouton Snapshot du DE400 nécessite toujours l'identification de son
signal sur le matériel ; aucun nouveau support matériel n'est annoncé ici.

### Revue complémentaire du plein écran et des commandes vidéo

Les outils d'image restent sous le direct, sur une ou deux lignes selon la
largeur disponible. En plein écran, les boutons superposés gardent une largeur
adaptée à leur texte : sortie de 36 pixels de haut à 16 pixels du bord droit,
capture de 48 pixels de haut à 20 pixels du bas. L'indicateur « Image figée » et
la sortie plein écran occupent deux lignes distinctes.

Le délai de disparition passe d'une à trois secondes. Le survol, un clic
maintenu et le focus clavier suspendent cette disparition ; le mouvement de
la souris ou `Tab` révèle les commandes. Quand la capture est désactivée, son
explication apparaît au-dessus du bouton. Rotation, miroirs et plein écran
disposent d'infobulles explicites. Dans la visionneuse, les boutons de zoom et
de fermeture sont centrés sur la ligne du titre ; lecture et timeline restent
sous la vidéo.

Les zones d'image du direct, de la visionneuse et des références disposent
chacune d'une liaison locale pour `Échap`. Elles partagent l'ordre de fermeture
avec le raccourci principal : visionneuse, références, réglages, puis plein
écran. La fermeture reste accessible lorsque l'image possède le focus.

Les tests du viewport couvrent les marges et les limites des zones cliquables
à 800 × 600, 1024 × 720, 1360 × 860 et 1920 × 1080, à 100 %, 125 %, 150 % et
200 %. Ils couvrent aussi l'attente avant disparition, le survol prolongé,
le clic maintenu, la navigation clavier et la perte du flux. Les **64 tests
applicatifs** passent après ces ajustements ; les **186 tests du workspace**
avaient également passé la validation complète des modifications visuelles.
Clippy strict sur tous les targets Linux passe à nouveau.

Les captures couvrent désormais **704 vues** et **144 comparaisons d'images**,
dont le plein écran photo/vidéo, les commandes masquées, la capture indisponible
et la visionneuse vidéo. Les comparaisons finales réutilisent les captures déjà
générées (`--compare-only`), sans les rendre une seconde fois. Les essais natifs
sous Xvfb utilisent des images synthétiques ; les temporisations attendent
l'affichage plutôt qu'un délai fixe trop court.

## Repères dans le code

- `crates/iriscope-core/src/library/` : transaction média, dossier et reprise.
- `crates/iriscope-core/src/library.rs` : API publique de bibliothèque.
- `crates/iriscope-core/src/file_validation.rs` : versions, SHA et cache de validation.
- `crates/iriscope-core/src/video.rs` : AVI, validation et récupération.
- `crates/iriscope-app/src/runtime.rs` : état partagé et fermeture des workers.
- `crates/iriscope-app/src/*_controller.rs` : callbacks de chaque fonction.
- `crates/iriscope-app/src/photo_worker.rs`, `recording_worker.rs`, `library_worker.rs` et `reference_worker.rs` : pipelines de fond.
- `crates/iriscope-app/src/library_ui.rs` : pagination, cache et images de référence.
- `crates/iriscope-camera-windows/src/` : capture et contrôles Media Foundation.
- `crates/iriscope-camera-macos/src/platform.rs` : sortie native AVFoundation.
- `scripts/package-release.py` et `.github/workflows/ci.yml` : distribution et vérifications.

## Images de référence

L'import des deux planches est disponible dans les réglages. Aucune paire adaptée n'a été retenue pour être livrée avec l'application. La [carte de Kampanis](https://commons.wikimedia.org/wiki/File:Iridology_chart.jpg) autorise la redistribution avec attribution, mais ne mesure que 371 × 368 pixels et représente un seul iris. Le [livre de Lindlahr de 1922](https://wellcomecollection.org/works/mb4p8y3e), indiqué dans le domaine public par Wellcome, offre des pages numérisées à haute résolution, mais reste une référence historique en anglais dont les illustrations n'ont pas été validées pour cet usage. Les planches modernes commerciales consultées n'annoncent pas de permission de redistribution vérifiée. Le choix de deux fichiers adaptés et autorisés reste donc nécessaire pour une distribution qui les inclut.


## 5 octobre 2026 — dossiers, recherche, sauvegarde et comparaison

- Recherche de captures par numéro de dossier, œil et période inclusive ; tri chronologique et maintien de la pagination avec validation des associations affichées.
- Contexte dossier/œil/mode visible en plein écran ; correction des noms sans changement de numéro ni renommage des médias.
- Navigation de la visionneuse entre les pages ; référence photo conservée et comparaison avec zoom et déplacement synchronisés, accompagnée des dates et dossiers.
- Sauvegarde complète et restauration vérifiée dans un nouveau dossier, hors du dossier source, avec refus des liens, chemins invalides et copies altérées. Les médias actuels ne sont pas remplacés.
- Alerte d’espace disque et vérification avant les écritures ; arrêt possible pendant les états de stockage critique.
- Outils de mesure en release, protocole de validation matérielle et préparation d’installateurs Debian, Windows et macOS. Le bouton physique Snapshot nécessite des essais sur l’ordinateur et la caméra réels.

Validation complémentaire dans le cloud Linux, avec des données fictives :

- **194 tests réussis**, aucun échec ; les deux essais matériels sont ignorés. Les 66 tests de l’application couvrent aussi les recherches invalides, les gardes de stockage, l’arrêt d’une vidéo, la comparaison, le blocage des captures pendant la correction d’un dossier et le retour du focus clavier après fermeture du dialogue.
- Clippy strict sur tous les targets du workspace, en natif Linux et pour Windows x86_64, macOS x86_64 et macOS ARM64 : réussi. Ces compilations croisées ne remplacent pas les essais sur les bureaux natifs.
- **800 vues synthétiques** et **192 comparaisons d’images**, à quatre tailles, deux thèmes et quatre échelles. La comparaison contrôle également la présence des deux images et leur séparation. Le message de capture en plein écran est placé sous les commandes et le contexte du dossier pour éviter leur chevauchement.
- Application réelle sous Xvfb : nom corrigé avec conservation du numéro et des trois associations ; recherche par date ; sauvegarde et restauration avec empreintes vérifiées ; copie altérée refusée sans remplacement des captures. Les retours de sélection sont simulés via `zenity` ; l’affichage des sélecteurs natifs n’est pas validé par cet essai.
- Navigation de la visionneuse avec 103 captures fictives : passage dans les deux sens entre la dernière capture de la première page et la première de la suivante, avec conservation de la recherche par dossier. Comparaison de deux photos, zoom et fermeture clavier vérifiés dans l’application.
- Mesures en release sur 1 000 et 10 000 captures, décodage JPEG et AVI synthétique d’une heure (28 800 images, environ 2,86 Go), avec contrôle de 100 positions de lecture. Résultats et limites dans `docs/PERFORMANCE.md`.
- Installateur Debian construit, empreinte et permissions vérifiées. Préparation des paquets Windows/macOS contrôlée avec fichiers fictifs, y compris le droit d’exécution du bundle macOS ; leurs outils natifs et l’installation sur ces systèmes restent à tester sur les hôtes correspondants. L’usage personnel retenu ne demande pas de signature officielle.

## 5 octobre 2026 — harmonisation des commandes et messages vidéo

- « Quitter le plein écran », le contexte dossier/œil/mode, le compteur de photos et les commandes de capture utilisent le même fond sombre, la police Manrope à 13 pixels et un poids de 600, avec des arrondis de 7 pixels.
- Les textes des boutons sont centrés sur toute leur hauteur. Les largeurs suivent le texte avec des marges internes identiques, sans les anciens espaces vides du compteur de photos.
- Les commandes courantes et les informations mesurent 36 pixels de haut ; la capture mesure 44 pixels pour conserver sa priorité. Les marges du plein écran sont de 16 pixels, avec 8 pixels entre les éléments d’une même ligne.
- La sortie du plein écran garde une position fixe. Le contexte, les états d’enregistrement et d’image figée, ainsi que le message de capture occupent des lignes distinctes.
- Les messages de capture sont placés dans la zone vidéo. Avec un nom de fichier long, « Voir » et la fermeture restent centrés verticalement. « Voir » ferme le message avant d’ouvrir la capture pour dégager les commandes de la visionneuse.
- L’installation personnelle sans signature officielle est documentée ; aucun certificat payant n’est nécessaire pour cette distribution.

Validation de cette harmonisation sous Linux : les **66 tests applicatifs** passent,
puis les cinq tests d’interaction sont revérifiés après le dernier ajustement du
message de capture. Le clic réel sur « Voir » est couvert avec un nom de fichier
long. Clippy strict sur tous les targets du workspace, le formatage Rust et le
contrôle des espaces passent également.

**800 rendus synthétiques** ont été générés dans les deux thèmes, à quatre tailles
de fenêtre et à 100 %, 125 %, 150 % et 200 %. Après revue des rendus, les
**208 comparaisons d’images** passent, avec de nouvelles références pour les
messages longs en mode fenêtré. Ces contrôles utilisent le rendu logiciel sous
Xvfb et ne constituent pas des essais sur les bureaux natifs Windows/macOS.

## 5 octobre 2026 — finalisation de la version personnelle 0.2.0

- Sauvegarde et restauration : progression par fichiers et octets, annulation pendant la copie ou l’attente des verrous, retrait des dossiers temporaires et fermeture qui attend leur nettoyage.
- La fermeture annule aussi les opérations encore en file d’attente avant l’ouverture d’un sélecteur, pour les sauvegardes et les exports.
- Dernière sauvegarde réussie conservée pour chaque dossier de captures récemment utilisé ; rappel facultatif après sept jours, désactivé par défaut, avec persistance après redémarrage.
- Export d’une copie MP4 H.264 depuis la visionneuse avec FFmpeg : progression, annulation, conservation de l’AVI, refus de remplacer un fichier existant et retrait des exports partiels en cas d’erreur. Une image JPEG corrompue fait échouer l’export au lieu d’être omise silencieusement.
- Paramètres : version affichée, commandes de sauvegarde et de rappel dimensionnées selon le texte ; annulation accessible dans la visionneuse et les paramètres.
- Paquets 0.2.0 sans signature officielle, avec version et empreinte du binaire, instructions de mise à jour et intégration facultative d’un FFmpeg autonome accompagné de sa licence. Les identifiants natifs et emplacements des réglages sont conservés ; les captures restent hors des fichiers installés.

Les **209 tests du workspace** passent sous Linux ; les deux essais nécessitant
la caméra restent ignorés. Les nouveaux essais couvrent l’interruption brutale
d’un processus puis une nouvelle sauvegarde/restauration, une destination en
lecture seule, la fermeture pendant une copie, les erreurs et l’annulation dans
l’interface, l’historique après redémarrage et l’encodage réel H.264 avec contrôle
de la durée et du nombre d’images. Les retours de sélection sont simulés via
`zenity` ; les sélecteurs natifs ne sont pas validés par ces tests.

Les **six contrôles de structure des paquets** passent, dont la construction et
l’extraction d’un vrai paquet Debian. Les outils Inno Setup et hdiutil sont
simulés pour contrôler les identifiants et permissions Windows/macOS.

Les références visuelles passent de 208 à **272 images**, avec les nouveaux
états de sauvegarde, rappel et export MP4. Les scénarios concernés sont rendus
à quatre tailles, dans les deux thèmes, de 100 % à 200 %. Les vues inchangées
sont réutilisées pour la comparaison complète ; la matrice entière comprend
désormais 896 vues synthétiques.

L’analyse Clippy stricte du workspace passe en natif Linux et en compilation
croisée pour Windows x86_64, macOS Intel et macOS Apple Silicon. Les derniers
contrôles d’export et du parcours graphique passent après les ajustements finaux.

La version Linux optimisée 0.2.0 est construite et distribuée localement en
archive portable et paquet Debian, avec empreintes SHA-256 vérifiées. Le binaire
extrait du paquet Debian démarre et se ferme sous Xvfb avec une ancienne
configuration : dossier des captures, modèle de nommage, thème, qualité vidéo,
préférence du bouton et valeurs caméra sont conservés ; la photo synthétique
existante reste identique. Le paquet ne contient que des fichiers sous `usr/`.

Les essais avec le DE400, le bouton physique et l’installation sur les bureaux
Windows/macOS restent à effectuer sur les ordinateurs correspondants, selon
`docs/VALIDATION-MATERIELLE.md`.

## 6 octobre 2026 — outils de consultation des photos

- Zoom continu de 10 à 800 %, taille réelle tenant compte de l’échelle du moniteur, zoom sous le curseur, déplacement, loupe ×2,5 et plein écran avec sortie par Échap.
- Notes de consultation et annotations manuelles : cercle, flèche, point et texte, aperçu pendant le tracé, masquage, suppression individuelle et annulation. Les notes restent modifiables pendant l’enregistrement automatique ; une erreur conserve le brouillon.
- Observations associées à l’empreinte de la photo, conservées dans l’index local et incluses dans les sauvegardes/restaurations. La navigation et la fermeture attendent leur enregistrement ; un fichier remplacé ne reprend pas les anciennes observations.
- Choix de photos du même dossier et du même œil, par période facultative ; cadrage indépendant et liaison qui conserve l’alignement. Références importées à côté de la photo et cercles repères réglables manuellement.
- Luminosité, contraste, rotation et miroir réversibles, avec retour à l’original. Copie PNG en pleine résolution avec annotations et réglages d’affichage ; fiche PDF avec l’original annoté, notes paginées et éventuelle comparaison.
- Originaux conservés, refus de remplacer les exports existants et nettoyage des copies temporaires lors de l’annulation. Les panneaux restent accessibles par défilement ; le bouton d’enregistrement des notes est visible dès l’ouverture à 800 × 600.

Les outils et leurs limites sont décrits dans `docs/VISIONNEUSE.md`. Les images
de référence et les repères servent à l’observation manuelle.

Validation sous Linux avec des captures fictives : les **219 tests du workspace**
passent ; les deux essais matériels restent ignorés. Après les derniers
ajustements, le test de géométrie est revérifié : taille réelle à 200 % d’échelle
du bureau, zoom sous le curseur, coordonnées après rotation/miroir, flèche
cantonnée à sa zone et loupe ×2,5 mesurée sur les pixels du rendu.
Le parcours applicatif vérifie aussi la saisie pendant une sauvegarde,
l’enregistrement avant navigation/fermeture, l’annulation d’une suppression,
la liaison des cadrages, les exports réels et la protection du brouillon après
remplacement de la photo source. Les retours des sélecteurs sont simulés via
`zenity` ; leurs fenêtres natives ne sont pas validées par cet essai.

**320 vues de la visionneuse ont été actualisées**, à quatre tailles, dans les
deux thèmes et aux quatre échelles de 100 % à 200 %, avec nouvelle revue des
panneaux, du zoom et de la loupe. Les vues inchangées sont réutilisées dans la
matrice de 1 088 captures ; les **384 comparaisons d’images** passent. Les PDF
d’essai sont également relus avec `pdfinfo` et `pdftotext` pour contrôler leur
structure et les textes accentués.

Clippy strict sur tous les targets du workspace : réussi en natif Linux et
pour Windows x86_64, macOS Intel et macOS Apple Silicon. Le formatage Rust et
le contrôle des espaces passent. La compilation croisée ne remplace pas les
essais sur les bureaux natifs. Les paquets 0.2.0 déjà générés ne sont pas
reconstruits par cet ajout des outils photo.

## 6 octobre 2026 — parcours de consultation, version 0.3.0

- Commandes **Comparer** et **Exporter…** séparées ; panneau **Annoter / Notes** explicite et état permanent des notes avec heure du dernier enregistrement.
- Choix de comparaison avec miniatures, date et œil, légendes lisibles sur plusieurs lignes ; marque **Retenue** conservée avec les observations, incluse dans les sauvegardes et actualisée au retour dans le dossier.
- Vue **Dossiers / Séances** : recherche par nom ou numéro, choix explicite du dossier, journées de captures, miniatures gauche/droite et notes générales distinctes des notes d’une photo. La journée actuelle est disponible avant une capture. Les notes générales se sauvegardent automatiquement, y compris avant fermeture ou changement de journée ; une erreur conserve le brouillon.
- Champs **Du / Au** avec dates françaises et calendrier partagé par la bibliothèque et la comparaison. Le format ISO reste accepté pour les anciennes saisies.
- Compteurs de photos près du choix de l’œil : conservés lors du changement de côté et remis à zéro lors du changement de personne ou de la fin de session. Le choix de l’œil reste manuel.
- Fermeture par Échap des dossiers et du calendrier, commandes de capture accessibles à petite taille et guides actualisés inclus dans les paquets 0.3.0.

Les **220 tests du workspace** passent sous Linux ; les deux essais nécessitant
une caméra réelle restent ignorés. Le parcours graphique vérifie notamment le
calendrier d’une année bissextile, Échap, la sélection gauche/droite et ses
miniatures, la marque retenue au retour d’une photo, la saisie pendant une
sauvegarde, le brouillon après échec, les notes générales avant fermeture et
la conservation des compteurs lors du changement d’œil. Le test du stockage
vérifie l’isolation par dossier et journée, les homonymes, les limites de notes,
l’annulation et leur récupération après sauvegarde/restauration.

Les **six contrôles de structure des paquets** passent, dont un vrai paquet
Debian extrait ; les outils d’installation Windows/macOS restent simulés dans
ces contrôles. Les originaux et les paramètres personnels restent hors des
fichiers installés. Le guide de consultation est dans `docs/VISIONNEUSE.md` et
les instructions de mise à jour dans `docs/RELEASE-0.3.0.md`.

Les **432 comparaisons visuelles** passent après mise à jour et revue des
rendus concernés. Les **864 captures fraîches** couvrent 27 scénarios,
quatre tailles de fenêtre, les deux thèmes et les échelles 100 %, 125 %,
150 % et 200 %. La matrice complète disponible comprend 1 184 vues, avec
les autres scénarios de la bibliothèque et des références. Le mode `--release`
du script accélère le rendu avec les mêmes scénarios et contrôles d’images.

Clippy strict sur tous les targets du workspace : réussi en natif Linux et
pour Windows x86_64, macOS Intel et macOS Apple Silicon. Le formatage Rust et
le contrôle des espaces passent. La version Linux optimisée 0.3.0 est
reconstruite avec l’ensemble des outils photo et du parcours de consultation.
La compilation croisée ne remplace pas les essais sur les bureaux natifs ;
le bouton physique reste à vérifier sur l’iriscope réel.

## Interface personnalisable · 6 octobre 2026 · version 0.4.0

Les préférences d’affichage sont locales, avec reprise des anciennes configurations et bornes de largeur. La disposition retrouve les panneaux patient/photo, les largeurs des trois séparateurs et la grille/liste. La réinitialisation conserve le thème et les données de consultation.

La présentation masque les informations privées et les annotations, sans effacer ni modifier les sources. Les notes continuent leur enregistrement ; les erreurs restent signalées sans chemin ni nom personnel. Image seule agrandit l’espace photo et masque les commandes après trois secondes sans interaction ; leur retour reste accessible à la souris et par Échap. Les miniatures ont trois tailles en grille et en liste. Le diagnostic caméra garde son état visible et replie les données techniques par défaut. Aucune option de masquage permanent de l’aide n’a été ajoutée.

Validation 0.4.0 : 223 tests de workspace réussis, deux essais caméra ignorés faute de matériel ; Clippy strict sur Linux, Windows x64 et macOS x64/ARM64. La matrice produit 1 536 rendus (48 scènes, quatre fenêtres, deux thèmes, quatre échelles) et valide 608 comparaisons de référence. Les essais couvrent les séparateurs à la souris, la reprise des préférences, le masquage des données, les raccourcis privés, les échecs de notes et l’ouverture du panneau mémorisé après lecture réelle de la photo et des observations. Six tests d’empaquetage passent ; les outils Windows/macOS sont simulés. Le paquet Debian démarre et se ferme normalement sous Xvfb, conserve les anciennes préférences et un original JPEG, et mémorise le panneau entre deux démarrages. L’archive portable, les ressources et les empreintes sont vérifiées. Les essais natifs Windows/macOS et du bouton physique restent à effectuer sur leurs appareils.
