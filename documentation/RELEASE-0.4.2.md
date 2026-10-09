# IrisScope 0.4.2

Cette correction conserve les mises à jour intégrées et le fonctionnement du
DE400 de la 0.4.1 sur macOS Intel et Linux x86_64.

- Après une mise à jour Linux portable, l’application redémarre dans le nouveau
  dossier. Le nettoyage de l’ancienne version ne laisse plus un dossier de
  travail supprimé, notamment lors d’un lancement depuis un terminal.
- L’installateur travaille dans son propre dossier temporaire, hors des fichiers
  de l’application qu’il remplace.
- Les paquets incluent automatiquement les notes de la version distribuée.

Depuis la 0.4.1, ouvrir **Paramètres**, télécharger la mise à jour, puis cliquer
sur **Installer et redémarrer**. Une authentification peut être nécessaire pour
approuver le nouveau paquet USB Mac ou installer le paquet Debian. Les ouvertures
suivantes sur Mac réutilisent l’autorisation USB.

Les paquets Mac Intel restent signés gratuitement ad hoc, sans notarisation.
Windows et Apple Silicon ne sont pas publiés. L’installation via Polkit/dpkg doit
encore être essayée sur un poste Linux ; les contrôles réalisés sur Mac et la
construction Linux ne remplacent pas cet essai.
