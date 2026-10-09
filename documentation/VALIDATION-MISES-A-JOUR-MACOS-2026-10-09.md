# Validation des mises à jour sur macOS Intel — 9 octobre 2026

Version 0.4.1, macOS 15.7.7 Intel, DE400 connecté. Les preuves et images
réelles restent dans `target/updater-release-20261009/`, hors Git.

## Code et interface

- Clippy strict en release passe sur tous les targets de l’application et de
  l’updater ; la dernière correction du redémarrage USB est aussi contrôlée.
- 69 tests de l’application et 8 tests de l’updater passent en release.
  La fermeture pour installer est refusée pendant un enregistrement, une sauvegarde ou des
  notes non enregistrées. Les signatures altérées, les archives tronquées,
  les liens, les versions anciennes et les autres tags sont refusés.
  Une erreur de relancement restaure l’ancienne application.
- La suite Python passe : 18 tests réussis et un ignoré. La construction Debian réelle
  nécessite `dpkg-deb`, absent du Mac. La structure Debian avec composant de
  mise à jour, encodeur optionnel et dépendances est contrôlée localement.
- Le site passe ses contrôles de liens et de ressources.
- 128 rendus des paramètres sont recalculés et comparés aux références
  approuvées : 8 scènes, deux tailles, deux thèmes, quatre échelles.
  Les états disponible, prêt à installer et hors ligne sont inspectés.
  32 rendus supplémentaires couvrent 1024 × 720 et 1920 × 1080 à 100 %.
  Les 848 autres références de l’interface restent celles déjà validées,
  sans recalcul inutile. Le dépôt contient maintenant 976 références.

Une passe de compilation optimisée du GUI, de l’updater et des rendus est
menée à terme après interruption d’une première passe pour corriger
l’environnement USB hérité. Une compilation du binaire de tests suit ;
les composants natifs USB déjà validés sont réutilisés sans recompilation.

## Installation réelle d’une mise à jour signée

La version installée 0.4.0 est remplacée par la 0.4.1 avec le même installateur
que celui des mises à jour intégrées. Le manifeste de cet essai local est signé
avec la véritable clé publique embarquée ; il contient seulement le paquet Mac.
Le manifeste public est produit ensuite avec les trois formats macOS/Linux.

L’essai injecte volontairement un ancien chemin de socket USB. Le relancement
supprime cette valeur et ouvre une nouvelle session USB. L’application confirme
le démarrage par son reçu, puis l’installateur supprime le dossier temporaire et
sa sauvegarde transactionnelle. Une copie de l’ancienne application est conservée
séparément dans le cache de validation.

Les 52 fichiers installés sont identiques au ZIP et au DMG signés. La signature
ad hoc est valide. Le JSON des réglages et les quatre médias existants sont
strictement identiques avant et après la mise à jour. L’autorisation USB du
nouveau paquet est approuvée avant l’essai ; les ouvertures suivantes utilisent
UID 501 sans nouvelle authentification.

## DE400 après la mise à jour

La validation utilise le véritable flux USB et le même gestionnaire de capture
que le bouton physique. Les commandes sont déclenchées par logiciel pour ce
complément ; aucun nouvel appui physique n’est demandé et les composants natifs
signés du lecteur et du service sont identiques au paquet précédent.

- Quatre PNG décodables de 1280 × 1024.
- Une vidéo AVI finalisée de 32 images à 8 fps.
- Yeux gauche/droit, deux dossiers homonymes et une capture sans dossier vérifiés.
- Sauvegarde et restauration de cinq médias et de leurs associations identiques.
- Neuf réglages caméra présents ; récepteur USB du bouton actif.
- L’export MP4 reste non testé sans FFmpeg optionnel.

La sortie arrête le lecteur USB et nettoie la session. L’application ordinaire
est relancée avec le service installé et la caméra connectée.

## Paquets vérifiés avant envoi

```text
GUI signé : c38ddd87390fafb1142bc45931e668f75b6592d454f50e1cfd155b04ca5e84d1
ZIP : e5b946679b051a47ba82d12f2c475fb02a0c525d95a84b2e274a888223a5ac0b
DMG : 9af0c6e0a71a1e8f5b16172b6eab86c31837fcf4db7ff177f3180443a34ac839
Mise à jour Mac : 9fc6eff584cc6b49a4c4628697bb9439ed6c74e519f3d3b35cd3f33b22c9e514
USB revision : d9c96e9226674a5db826b1bb9c0025d6ae1c1d4d643c672bdb2d99105b187f0c
```

GitHub reçoit les sources sur `main` et les paquets Mac ainsi vérifiés. Le seul
workflow réutilise ces paquets sans recompilation, construit Linux nativement
et publie la release seulement lorsque tous les fichiers et le manifeste signé
sont prêts. Aucun test ni lint n’est exécuté sur GitHub. Windows et Apple Silicon
ne sont pas publiés. L’installation réelle via Polkit/dpkg sur Linux reste à
valider sur ce système ; sa compilation ne constitue pas cet essai.
