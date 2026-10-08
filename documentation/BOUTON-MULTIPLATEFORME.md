# Bouton DE400 et développement depuis Linux

État au 7 octobre 2026. Les contrôles de compilation ne remplacent pas un essai avec la caméra sur le système concerné.

| Système | Réception du bouton | Validation réelle |
| --- | --- | --- |
| Linux | Passerelle `usbmon` et détection des appuis/relâchements | Photos et démarrage/arrêt vidéo validés sur le DE400 branché |
| Windows | Abonnement natif optionnel `IKsControl::KsEvent`, événement `KSEVENT_VIDCAPTOSTI_EXT_TRIGGER` | À réaliser ; le pilote doit exposer cet événement |
| macOS | Composant USB vidéo/bouton/réglages inclus dans le paquet ; AVFoundation disponible séparément | Le chemin ordinaire est refusé (`0xe00002c5`). Le chemin direct avec authentification administrateur est validé sur Mac Intel / macOS 15.7.7 : deux photos et démarrage/arrêt vidéo par quatre appuis réels |

Le bouton de capture à l'écran et les raccourcis utilisent le même traitement photo/vidéo sur les trois systèmes. Ce traitement est distinct de la réception du bouton matériel.

## Signal observé sous Linux

L'exemplaire `21cd:603b`, série `VTU603EB`, annonce `bTriggerUsage = 0`. Une pression produit `02 01 00 01` sur l'endpoint d'interruption `0x81`, puis `02 01 00 00` au relâchement. La passerelle filtre ce signal et transmet une seule capture par appui, sans ouvrir un autre flux caméra. Voir le [rapport Linux](VALIDATION-LINUX-2026-10-06.md) et les [instructions d'installation](../helpers/linux/README.md).

## Windows

Le backend essaie de s'abonner au [déclencheur externe documenté par Microsoft](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/ksevent-vidcaptosti-ext-trigger) sur la source Media Foundation déjà ouverte. Un événement Windows auto-réinitialisé reçoit les notifications ; la file de boutons est indépendante de la dernière image du direct. L'abonnement est retiré à la fermeture, et les événements anciens sont retirés lors d'un arrêt/redémarrage du flux.

Un pilote peut refuser l'abonnement ou ne jamais produire cet événement pour ce firmware. Le `bTriggerUsage = 0` observé sous Linux rend cet essai particulièrement nécessaire. Le code compile pour Windows x86_64, mais le fonctionnement du bouton DE400 sous Windows n'est pas confirmé. Aucun remplacement de pilote ni prise exclusive de l'interface USB n'est réalisé.

Le diagnostic indique désormais « abonnement natif Windows actif » lorsque le pilote accepte l’abonnement, ou fournit l’erreur réelle en cas de refus. Un abonnement actif ne confirme pas encore l’arrivée d’une pression : il faut exécuter l’essai avec le bouton physique.

## macOS

L'essai du 7 octobre 2026 avec le DE400 `21cd:603b`, série `VTU603EB`, sur un
MacBookPro12,1 sous macOS 15.7.7 confirme le refus de l'interface du bouton
(`could not open interface for exclusive access`, `0xe00002c5`). Le direct et
les captures logicielles restent disponibles, mais le premier essai physique
avec ce chemin ordinaire n'a produit aucune photo.

**Un second chemin, USB direct, fonctionne sur ce même Mac.** Une réservation
temporaire de l'iriscope avec les droits administrateur permet de recevoir
les paquets `02 01 00 01` et `02 01 00 00`. Comme cette réservation retire aussi
le pilote vidéo Apple, le composant reçoit lui-même le flux UVC et transmet
les images et événements à IrisScope, dont l'interface reste sous le compte
utilisateur. Quatre appuis réels enregistrent deux photos 1280 × 1024 puis
démarrent et arrêtent une vidéo de 40 images lisibles sur le paquet release
signé localement et installé depuis le DMG. Le scénario physique,
les associations aux dossiers/yeux et la sauvegarde/restauration passent.

Le nouveau paquet inclut un lanceur natif et le composant USB ; il recherche le
modèle `21cd:603b` sans imposer la série ni l'emplacement USB. Le mode direct est
1280 × 1024 YUYV à 8 fps. Les huit réglages annoncés par l'appareil sont exposés
et testés : luminosité, contraste, saturation, teinte, netteté, gamma, balance
des blancs et anti-scintillement, avec modification, relecture, réinitialisation
et restauration pendant la vidéo. La caméra peut être rouverte au cours d'une
même session autorisée. La session n'a pas de limite de durée arbitraire.

Le paquet reçoit une signature locale ad hoc gratuite, sans notarisation.
Sur macOS 15+, le premier lancement installe un service USB protégé après
authentification administrateur. L'autorisation est ensuite conservée : le
lecteur démarre à la demande et s'arrête à la fermeture de l'application.
Aucun mot de passe ni pilote Apple modifié n'est installé. Une mise à jour du
lanceur ou des composants USB peut nécessiter une nouvelle autorisation.
Voir [Autorisation USB conservée](ACCES-USB-MACOS.md).
Le lancement, les sondes et le retour vérifié à AVFoundation sont détaillés dans
le [rapport Mac](VALIDATION-MACOS-2026-10-07.md). Les autres versions de macOS et
les Mac Apple Silicon ne sont pas validés physiquement.

Le chemin ordinaire conserve AVFoundation pour le flux vidéo. Un thread natif utilisant IOKit, via `nusb`, recherche le même DE400 avec son emplacement USB encodé dans l’identifiant AVFoundation. Il ouvre uniquement l’interface de contrôle vidéo et lit l’endpoint d’interruption `0x81`, avec les paquets mesurés sous Linux. L’appui est dédoublonné jusqu’au relâchement ; sa file reste indépendante des images. Le récepteur est arrêté et les notifications sont effacées à chaque arrêt ou redémarrage du flux.

Ce lecteur ordinaire utilise l’ouverture USB normale : aucun détachement de pilote, aucune saisie du périphérique, aucun changement de configuration USB ni installation système. **Si le pilote Apple détient déjà cette interface, macOS peut refuser son ouverture**. La caméra continue alors de fonctionner et le diagnostic indique le refus ; ce refus ne se corrige pas par une simple permission caméra. Le chemin direct réserve temporairement le périphérique entier, puis le rend à macOS.

Dans **Paramètres → Diagnostic**, « réception USB macOS active » signifie que l’endpoint a été ouvert et que les lectures sont soumises. Un message « indisponible » ou « lecture interrompue » fournit l’erreur réelle. Une pression physique reste nécessaire pour confirmer l’arrivée d’un paquet. Le décodeur, l’association au bon emplacement USB et la file d’événements sont testés sous Linux ; la couche IOKit est contrôlée par compilation pour Mac Intel et Apple Silicon.

## Un seul poste de développement

Les modifications des trois backends, de l'interface et des tests se font dans le dépôt Linux. `scripts/ci-local.sh` contrôle le code des cibles Windows x86_64, Mac Intel et Mac Apple Silicon depuis Linux. La validation complète GitHub, lancée depuis **Actions → CI → Run workflow** avec **full_validation** cochée ou par un tag `v*`, construit et teste aussi l'application sur des machines Linux, Windows et macOS ; elle produit leurs paquets natifs. Les modifications du backend Mac ou de `scripts/macos/` lancent aussi cette validation native complète. Les autres CI automatiques légères vérifient seulement les fichiers modifiés et ne produisent pas de paquet. GitHub ne possède pas l'iriscope physique.

Un Mac et un PC Windows sont utiles uniquement pour installer le paquet et faire l'essai matériel, sans installer un environnement de développement. Après fermeture des autres applications utilisant la caméra, lancer l'exécutable natif avec `--diagnose`, puis `--validate-button` suivi d'un chemin vers un dossier vide. Sur macOS, vérifier d’abord l’état du récepteur dans le diagnostic. Si l’accès USB est refusé, le scénario de bouton ne peut pas réussir ; `--validate-hardware` vérifie séparément les captures logicielles. Le scénario crée des dossiers fictifs et conserve les paramètres personnels. Voir le [protocole matériel](VALIDATION-MATERIELLE.md).

Les commandes d’installation et d’essai sont détaillées dans [Essayer sur Windows et macOS](ESSAIS-WINDOWS-MACOS.md). Sans FFmpeg, le scénario indique que l’export MP4 n’a pas été testé, puis vérifie quand même les captures AVI et leur sauvegarde/restauration. Une autre erreur d’export reste un échec.
