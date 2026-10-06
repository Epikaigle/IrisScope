# Mesures reproductibles

Les commandes suivantes créent uniquement des données synthétiques temporaires ; elles ne lisent pas le dossier de captures de l’utilisateur.

```bash
cargo run --release -p iriscope-core --example benchmark_library --locked -- 1000
cargo run --release -p iriscope-core --example benchmark_library --locked -- 10000
cargo run --release -p iriscope-imaging --example benchmark_decode --locked
```

Le premier outil mesure le scan des métadonnées, la sélection d’une page filtrée et la vérification de 100 captures. Chaque fichier contient 64 Kio de données synthétiques ; cet outil ne décode pas les images. Le second décode 100 JPEG synthétiques de 1280 × 1024 pixels avec un contenu bruité pour solliciter le décodeur. Il inclut le coût d’allocation et affiche la médiane et le 95e percentile.

Mesures du 5 octobre 2026 dans l’environnement cloud Linux, en release :

| Charge | Scan des métadonnées | Sélection de 100 | Vérification de 100 |
|---|---:|---:|---:|
| 1 000 captures | 16,29 ms | 0,02 ms | 22,69 ms |
| 10 000 captures | 362,89 ms | 1,11 ms | 132,42 ms |

JPEG bruité de 2 070 799 octets : décodage médian 44,05 ms, 95e percentile 57,33 ms. Ces résultats dépendent du matériel, du stockage et des autres tâches en cours. Ils ne mesurent pas la latence du direct ni la cadence soutenue du DE400.

Pour ces dernières mesures, utiliser le binaire release et le protocole de [validation matérielle](VALIDATION-MATERIELLE.md). Observer la mémoire pendant 30 minutes, pendant l’enregistrement et après navigation répétée dans de grandes bibliothèques. Les budgets de queues et de caches sont déjà bornés ; toute hausse continue doit être analysée avant d’ajouter un autre cache.

Pour mesurer une vidéo longue, fournir un JPEG **synthétique** de 1280 × 1024 pixels, inférieur à 128 Kio :

```bash
cargo run --release -p iriscope-core --example benchmark_video --locked -- /chemin/image-synthetique.jpg
```

L’outil crée un AVI temporaire d’une heure à 8 FPS, contrôle ses 28 800 images et vérifie 100 positions de lecture grâce à un identifiant dans chaque image. Il vérifie l’espace libre avant de créer le fichier et supprime ses données temporaires après l’essai. Pour le JPEG synthétique de 99 302 octets utilisé ici, le fichier fait 2 860 821 256 octets : indexation 100,53 ms, lecture d’une image après déplacement médiane 0,04 ms et 95e percentile 0,15 ms. Ces temps couvrent le conteneur et les lectures, pas le décodage, le rendu ni un enregistrement matériel en temps réel.
