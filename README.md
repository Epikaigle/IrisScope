# IrisScope

IrisScope est une application desktop multiplateforme destinée à l'utilisation
d'un iridoscope Firefly DE400 / Infoxelle Digital Microscope.

## État du développement

Les étapes 1 à 4 mettent en place le workspace Rust partagé, sa validation
locale, l'abstraction caméra commune et la détection native. Les crates sont
séparées selon leurs responsabilités afin de garder le cœur métier indépendant
des API caméra natives :

- `iriscope-core` : modèles et logique métier partagés ;
- `iriscope-camera-linux` : backend V4L2 ;
- `iriscope-camera-windows` : backend Media Foundation ;
- `iriscope-camera-macos` : backend AVFoundation ;
- `iriscope-imaging` : décodage et traitement d'image ;
- `iriscope-renderer` : rendu du flux caméra ;
- `iriscope-app` : point d'entrée de l'application.

Les backends caméra, Slint et le rendu GPU seront ajoutés dans les étapes
fonctionnelles correspondantes.

L'interface `CameraBackend` couvre l'énumération et le hotplug. Une caméra
ouverte expose ses capacités réelles, le démarrage du flux, les frames natives,
les contrôles et les événements du bouton physique à travers `CameraDevice`.

L'énumération utilise V4L2 sous Linux, Media Foundation sous Windows et
AVFoundation sous macOS. Le backend Linux ignore les nœuds de métadonnées et
associe les nœuds de capture à l'identité USB trouvée dans sysfs.

## Validation locale

Les vérifications sont exécutées sur la machine de développement avec :

```text
./scripts/ci-local.sh
```

Ce script vérifie le formatage, compile, exécute Clippy et lance les tests sur
Linux. Il contrôle aussi par compilation croisée les cibles Windows x86_64,
macOS Intel et macOS Apple Silicon. Les binaires natifs Windows et macOS devront
être exécutés sur les machines correspondantes lorsqu'elles seront disponibles.

Pour inclure la détection de la caméra branchée :

```text
./scripts/ci-local.sh --hardware
```

## Vérification locale

```text
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
