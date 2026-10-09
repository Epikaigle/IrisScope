# IrisScope 0.4.3

Le logo d’iris doré de l’interface devient l’icône de l’application, sur un
fond blanc fixe, quel que soit le thème du système.

- macOS Intel : icône haute résolution intégrée au paquet, visible dans le
  Finder, le Dock et le sélecteur d’applications.
- Linux : même icône pour la fenêtre et le paquet Debian. L’identifiant de
  l’application correspond à son entrée de menu sur X11 et Wayland.
- La caméra, le bouton USB et les données conservent le fonctionnement de la 0.4.2.
- Deux téléchargements seulement : **macOS Intel `.dmg`** et **Linux amd64 `.deb`**.
  Ces mêmes installateurs servent aussi aux mises à jour intégrées. Les signatures
  et les empreintes restent vérifiées, sans fichiers techniques supplémentaires
  dans la liste de téléchargement.

Depuis une 0.4.1 ou 0.4.2, installer cette version une fois manuellement.
À partir de la 0.4.3, les versions suivantes s’installent depuis **Paramètres**. Une
authentification peut être nécessaire pour approuver le nouveau paquet Mac ou
installer le Debian. La signature Mac reste gratuite ad hoc, sans notarisation.
Windows et Apple Silicon ne sont pas publiés dans cette release.
