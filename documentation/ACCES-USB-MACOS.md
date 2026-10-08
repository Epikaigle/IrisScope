# Autorisation USB conservée sur macOS

Le paquet DE400 pour **macOS 15 ou plus récent** inclut le service USB et son
installateur. Le premier lancement demande l'authentification administrateur
native de macOS. Après cette installation, l'ouverture d'IrisScope démarre le
lecteur USB à la demande, sans ressaisir le mot de passe. Aucun abonnement Apple,
certificat payant ou mot de passe enregistré n'est nécessaire.

## Installation et fonctionnement

1. Copier `IrisScope.app` depuis le DMG dans Applications et ouvrir l'application.
2. Autoriser la fenêtre d'authentification macOS au premier lancement.
3. Utiliser la vidéo, les huit réglages annoncés par le DE400 et son bouton.
4. Fermer IrisScope : le lecteur libuvc s'arrête et rend la caméra au pilote Apple.

Il n'y a pas de durée maximale de session. Le service installé n'ouvre pas la
caméra pendant l'inactivité ; il n'enregistre aucune image. L'interface et les
fichiers photo/vidéo restent sous le compte utilisateur. Le mode direct reste
1280 × 1024 YUYV à 8 fps.

L'installation persiste après fermeture et redémarrage du Mac. Une modification
du lanceur ou des composants USB peut demander une nouvelle autorisation pour
mettre à jour les fichiers protégés. Si le service est désactivé ou arrêté,
IrisScope propose une réparation explicite ; un refus ne déclenche pas de capture
silencieuse avec un bouton inutilisable.

## Composant installé et droits

- `/Library/Application Support/IrisScopeUSB/current/` contient seulement le
  service, le lecteur DE400 et leur manifeste. Le dossier et ses fichiers sont
  détenus par root et ne sont pas modifiables par un utilisateur ordinaire.
- `/Library/LaunchDaemons/app.iriscope.usb-service.plist` déclare le service
  `launchd` à la demande. Aucune modification de `sudoers`, des pilotes Apple ou
  de SIP n'est effectuée.
- L'application et le service vérifient mutuellement leurs exigences de signature
  sur les messages XPC. Avec la signature ad hoc gratuite, le manifeste protégé
  réserve l'accès au lanceur précis approuvé à l'installation.
- Le client ne fournit ni commande, ni chemin de programme, ni UID à exécuter.
  Le service obtient l'identité du client auprès de macOS et lance uniquement
  son lecteur DE400 installé. Ce lecteur ne recherche que l'appareil `21cd:603b`.
- Chaque réception suit le PID et la date de démarrage de son lanceur. Les sockets
  sont privées au compte utilisateur. L'arrêt de la connexion XPC nettoie la
  réception ; aucune limite horaire n'est utilisée.
- Le journal technique `/Library/Logs/IrisScopeUSB.log` est réservé à root ;
  il contient les états USB et les compteurs, sans les images ni les dossiers.

La signature ad hoc assure l'intégrité locale ; elle n'est pas une identité
Developer ID ni une notarisation. L’API XPC employée existe dès macOS 13 ; les bibliothèques USB et le lecteur
des paquets distribués ciblent macOS 15, minimum déclaré dans le paquet.
Le chemin gratuit retenu utilise un service
`launchd` installé après authentification, plutôt que de supposer qu'une
signature autosignée suffit à l'enregistrement d'un daemon SMAppService.

## Diagnostic et désinstallation

Ces commandes sont disponibles dans le paquet, sans Rust ni Python :

```sh
/Applications/IrisScope.app/Contents/MacOS/IrisScope --usb-service-status
/Applications/IrisScope.app/Contents/MacOS/IrisScope --install-usb-service
/Applications/IrisScope.app/Contents/MacOS/IrisScope --uninstall-usb-service
```

Le statut n'ouvre aucun périphérique et ne demande pas d'authentification.
L'installation déjà conforme ne redemande pas le mot de passe. La désinstallation
requiert une authentification et supprime uniquement le service et son
enregistrement ; l'application et les données patient sont conservées.
Le journal technique et le fichier vide de verrouillage de l'installateur restent
sur le Mac ; ils n'ouvrent pas de périphérique et ne contiennent pas de mot de passe.
Supprimer uniquement l'app du Finder ne désinstalle pas ce service local.

`--temporary-usb` conserve le lancement ponctuel du lecteur avec une
authentification par ouverture, pour les diagnostics. `--avfoundation` utilise
le pilote vidéo Apple ; ce chemin n'a pas récupéré le bouton du DE400 testé.

## Développement sur un Mac lent

Si seul le service ou le lanceur change, réutiliser la compilation release Rust
et les bibliothèques USB existantes :

```sh
python3 scripts/macos/build-usb-experiment.py --service-only
python3 scripts/package-release.py
python3 scripts/package-installer.py
```

Le premier script ne compile que deux petits programmes Objective-C. La réception
libuvc et l'exécutable Rust déjà optimisé sont réutilisés. Une modification du
lecteur C exige sa recompilation ; une modification fonctionnelle Rust exige
les vérifications Rust appropriées.
`--reader-only` recompile seulement le lecteur C et son objet de redistribution
avec les bibliothèques USB déjà construites, sans relancer CMake ou Cargo.

## Sources de l'architecture

- [Apple : services launchd à la demande](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html).
- [Apple : exigences de signature sur les connexions XPC](https://developer.apple.com/documentation/foundation/nsxpcconnection/setcodesigningrequirement(_:)).
- [libusb : accès Mac à un appareil détenu par un pilote](https://github.com/libusb/libusb/wiki/FAQ).

Les résultats matériels de cette variante doivent être consignés séparément de
ceux du lancement ponctuel validé le 7 octobre 2026.
