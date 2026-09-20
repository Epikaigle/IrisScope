# IrisScope

IrisScope est une application desktop multiplateforme destinée à l'utilisation
d'un iridoscope Firefly DE400 / Infoxelle Digital Microscope.

## État du développement

Les étapes 1 à 3 mettent en place le workspace Rust partagé, sa validation
continue et l'abstraction caméra commune. Les crates sont séparées selon leurs
responsabilités afin de garder le cœur métier indépendant des API caméra
natives :

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

## Intégration continue

Le workflow GitHub Actions vérifie chaque push et chaque pull request sur :

- Linux x86_64 ;
- Windows x86_64 ;
- macOS Intel ;
- macOS Apple Silicon.

Il compile et teste le workspace avec Rust 1.89.0. Un job Linux séparé vérifie
également le formatage et exécute Clippy avec les avertissements traités comme
des erreurs.

## Vérification locale

```text
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
