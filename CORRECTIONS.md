# Suivi des corrections et de la structure

Suivi du travail des 30 septembre et 1er octobre 2026. Les modifications sont dans le répertoire de travail ; ce document distingue les corrections de code et leur validation sur du matériel réel.

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
