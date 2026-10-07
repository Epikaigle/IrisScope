# Site de présentation et GitHub Pages

Le site statique se trouve dans `docs/`. Il présente l’application Iriscope,
ses écrans et les liens vers les paquets d’essai. La reconstitution 3D interactive
du DE400 reste à l’accueil comme illustration ; il n’y a plus de simulateur ni
de section de présentation commerciale de l’appareil. Les guides techniques se
trouvent dans `documentation/`, hors du dossier publié.

## Publier sur GitHub.io

Tout reste sur `main`, sans branche `gh-pages` :

1. Dans le dépôt **Epikaigle/iriscope-app**, ouvrir **Settings → Pages**.
2. Dans **Build and deployment → Source**, choisir **Deploy from a branch**.
3. Sélectionner la branche **main**, puis le dossier **/docs** et cliquer sur
   **Save**.
4. Après le déploiement, ouvrir l’adresse affichée dans **Settings → Pages** :
   `https://epikaigle.github.io/iriscope-app/`.

`docs/index.html` est l’entrée du site. Le fichier vide `docs/.nojekyll` indique
à Pages de servir les fichiers statiques sans transformation Jekyll. Seul
`docs/` est publié ; les guides, le code de l’application, les captures locales,
les diagnostics et les paramètres restent en dehors de ce dossier.

Une fois cette option activée, les changements poussés sur `main` sont publiés
automatiquement par GitHub Pages. Son déploiement apparaît dans Actions sous
**pages build and deployment** ; ce run est normal et distinct de la CI de tests.
Il n’y a aucun workflow de publication personnalisé à lancer.

La [documentation GitHub Pages](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site)
confirme que ce mode accepte la racine ou `/docs` comme dossier source.

## Aperçu local

Depuis la racine du dépôt :

```bash
python3 -m http.server 8765 --bind 127.0.0.1
```

Ouvrir `http://127.0.0.1:8765/docs/`. Les modules JavaScript nécessitent un
serveur HTTP ; l’ouverture directe par `file://` ne convient pas.

```bash
python3 scripts/check-site.py
```

Node.js doit être installé pour ce contrôle local de syntaxe JavaScript ; le
workflow GitHub **CI** prépare automatiquement Node.js et Python.

Ce contrôle vérifie la syntaxe JavaScript, les images, imports, fontes, liens
internes et identifiants d’accessibilité. Il refuse les chemins commençant par
`/`, qui feraient perdre le préfixe `/iriscope-app/` de GitHub Pages, et les ressources
de rendu chargées depuis un autre site. La seule CI de tests lance ce contrôle
pour les modifications de `docs/` ou de `scripts/check-site.py`. Un changement
limité au site ne lance pas Rust, les captures de l’application ou la création
des installateurs. Un changement limité à `documentation/` ne lance aucune CI.
Une revue dans un navigateur reste nécessaire pour les gestes, le cadrage 3D et
les petits écrans.

## Organisation

- `.nojekyll` : publication statique sans Jekyll ;
- `index.html` : contenu et structure accessibles ;
- `styles.css` : styles, composants et adaptations aux petits écrans ;
- `main.js` : navigation mobile et onglets des captures d’écran ;
- `three-scene.js` : géométrie du DE400 et commandes de la vue 3D ;
- `assets/` : photographies, captures d’écran, icône et fontes locales ;
- `vendor/` : Three.js et sa licence.

Toutes les ressources de rendu sont locales : pas de CDN, de traceur, de compte
à créer, de serveur applicatif ni de compilation nécessaire pour publier.
Les liens de téléchargement renvoient à GitHub Actions. Les paquets sont dans
**Artifacts** d’une validation complète réussie ; une CI automatique légère
n’en produit pas. Pour en générer de nouveaux, ouvrir **Actions → CI → Run
workflow**, sélectionner `main` et laisser **full_validation** cochée. Le
[guide Windows et macOS](ESSAIS-WINDOWS-MACOS.md) détaille leur choix et les essais.

## Modèle 3D et images

Le modèle est reconstruit à partir des photographies et proportions publiées
par [Firefly Global](https://fireflyglobal.com/de400-eyescope/). Ce n’est pas un
fichier CAO du fabricant. Il représente le corps noir, l’œilleton opaque,
l’optique et quatre LED, la molette argentée, le bouton Snapshot arrière, la
commande d’éclairage latérale et le câble USB. La 3D illustre l’application ;
elle ne constitue pas une fiche commerciale du matériel.

La rotation se commande à la souris, par glissement horizontal tactile ou avec
les flèches du clavier. Les boutons +/− règlent la distance et **Recentrer**
rétablit le cadrage. La rotation automatique est volontaire ; elle est arrêtée
si la préférence de réduction des animations est activée. Le rendu reste
statique au repos et s’arrête hors écran ou lorsque l’onglet est masqué. Sans
WebGL, la photographie remplace la 3D et le reste du site demeure utilisable.

Les photos de produit et d’iris fournies dans les ressources du site sont des
images de démonstration Firefly Global, créditées dans la page. Elles ne sont
pas des captures d’un patient de cette installation. Les droits des photos et
la marque Firefly restent ceux de leur propriétaire. Les fontes ont leurs
licences OFL dans `assets/fonts/`, Three.js sa licence MIT dans `vendor/`.

Les trois captures d’écran sont générées à partir de l’interface actuelle,
avec une image de démonstration et des données fictives :

```bash
cargo build --release -p iriscope-app --example ui_snapshots --locked
IRISCOPE_SNAPSHOT_IMAGE="$PWD/docs/assets/de400/iris-sample-1.jpg" \
IRISCOPE_SNAPSHOT_SCENES=camera-active,viewer-photo,library-custom \
SLINT_BACKEND=winit-software SLINT_SCALE_FACTOR=1 \
xvfb-run -a target/release/examples/ui_snapshots target/site-showcase
```

Copier les fichiers `*-1360x860-dark.png` concernés dans `docs/assets/app/`.
Les tests de comparaison visuelle ignorent ce paramètre et conservent leur
image déterministe habituelle.

## Mettre à jour Three.js

Version intégrée : **0.186.1**, bundle ESM produit avec **esbuild 0.28.2**.
Les sources amont sont disponibles dans [Three.js](https://github.com/mrdoob/three.js).
Pour reconstruire la même version sans ajouter d’outil au projet :

```bash
mkdir -p target/site-vendor
npm pack three@0.186.1 --pack-destination target/site-vendor
tar -xzf target/site-vendor/three-0.186.1.tgz -C target/site-vendor
npm exec --yes --package=esbuild@0.28.2 -- esbuild \
  target/site-vendor/package/build/three.module.js \
  --bundle --minify --format=esm --target=es2022 \
  --outfile=docs/vendor/three.module.js
cp target/site-vendor/package/LICENSE docs/vendor/THREE-LICENSE.txt
```
