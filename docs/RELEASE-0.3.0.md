# IrisScope 0.3.0 — consultation des photos

Cette version rassemble les outils photo ajoutés depuis 0.2.0 et les améliorations du parcours de consultation :

- zoom 10–800 %, taille réelle, loupe, plein écran, annotations manuelles et notes par photo ;
- comparaison avec miniatures et marque persistante **Retenue** ; commandes **Comparer** et **Exporter…** séparées ;
- copie PNG annotée et fiche PDF avec notes et éventuelle comparaison ;
- recherche des dossiers par nom ou numéro, journées avec photos gauche/droite et notes générales séparées ;
- dates françaises avec calendrier, état des sauvegardes de notes et compteur de captures par œil.

Le [guide de la visionneuse et des dossiers](VISIONNEUSE.md) explique les commandes et leurs limites. Les outils servent à l’observation manuelle, sans interprétation automatique.

## Installer ou mettre à jour

Fermer IrisScope, sauvegarder le dossier de captures complet avec ses fichiers cachés, puis installer le paquet ou remplacer l’application portable. Les paramètres et les captures restent hors du programme installé. Les identifiants des installateurs et l’emplacement des paramètres sont conservés : `%APPDATA%/IrisScope/settings.json` sous Windows, `~/Library/Application Support/IrisScope/settings.json` sous macOS et `$XDG_CONFIG_HOME/IrisScope/settings.json` ou `~/.config/IrisScope/settings.json` sous Linux.

Le numéro apparaît dans les paramètres, `VERSION.txt`, `release-info.json` et `iriscope-app --version`. Vérifier l’empreinte SHA-256 fournie avec le paquet. La distribution personnelle reste sans signature officielle.

Les notes et les marques de sélection sont conservées dans l’index local et incluses dans les sauvegardes. Les originaux ne sont pas réécrits. Conserver la sauvegarde avant mise à jour permet aussi de revenir à l’ancienne version avec son index correspondant.

## Plateformes et appareil

L’environnement cloud Linux permet de construire l’archive portable Linux et le paquet Debian. Les installateurs Windows et macOS doivent être construits sur leurs systèmes natifs ; la compilation croisée ne valide pas leur installation ni leur rendu de bureau.

Les exports MP4 utilisent FFmpeg avec `libx264`, installé séparément ou intégré avec sa licence lors de l’empaquetage. Sans FFmpeg, la capture et la lecture AVI restent disponibles.

Le [protocole matériel](VALIDATION-MATERIELLE.md) reste à effectuer sur l’ordinateur et l’iriscope réels. Le bouton physique sera testé sur cet appareil.
