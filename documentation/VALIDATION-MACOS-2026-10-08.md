# Validation Mac du service USB conservé — 8 octobre 2026

Cette validation complète le [rapport du 7 octobre](VALIDATION-MACOS-2026-10-07.md).
Elle concerne le DE400 USB `21cd:603b`, connecté au MacBookPro12,1 Intel sous
macOS 15.7.7. Les captures utilisent des dossiers de test distincts des données
patient. Aucun résultat matériel Windows ou Apple Silicon n'est déduit de cet essai.

## Résultat et installation

Le paquet inclut un service `launchd` installé après authentification
administrateur. Le lanceur normal établit ensuite sa connexion XPC et démarre
le lecteur DE400 à la demande, sans authentification par ouverture. Aucun
mot de passe n'est conservé dans le code, le paquet, la configuration ou les
journaux. L'interface et les captures restent sous le compte utilisateur.

Installation, désinstallation, réinstallation et vérification d'une installation
déjà conforme passent sur le Mac. La désinstallation ne supprime pas
l'application ou les données patient. Le statut et l'installation déjà conforme
s'exécutent sous le compte utilisateur, sans authentification. Après arrêt du
service par `launchctl`, une requête de statut le redémarre à la demande sans
mot de passe. Aucun redémarrage complet du Mac n'a été effectué pour ce test.

Les fichiers du service, son manifeste et sa déclaration `LaunchDaemons` sont
possédés par root et non modifiables par un utilisateur ordinaire. Le service
n'ouvre pas la caméra pendant l'inactivité. L'autorisation et le fonctionnement
sont détaillés dans [Accès USB macOS](ACCES-USB-MACOS.md).

## Bouton, vidéo et réglages

L'opérateur a effectué quatre appuis physiques dans l'application release :

- Une photo gauche de 1280 × 1024, associée au premier dossier de test.
- Une photo droite de 1280 × 1024, associée au deuxième dossier portant le même nom.
- Un appui pour démarrer la vidéo, puis un appui pour l'arrêter : AVI de 53 images
  MJPEG intégralement décodables à une cadence déclarée de 8 fps, associé au
  premier dossier et à l'œil droit.

Le lecteur a reçu quatre pressions et quatre relâchements, 133 images complètes
et aucune image incomplète lors de cet essai. La sauvegarde/restauration conserve
les trois médias octet pour octet avec leurs associations.

Les huit réglages annoncés sont affichés dans l'interface : luminosité, contraste,
saturation, teinte, netteté, gamma, température des blancs et anti-scintillement.
Un essai séparé, fenêtre ouverte pendant toute sa durée, vérifie pour chacun
modification, relecture, restauration et refus d'une valeur hors limites. Les
huit valeurs par défaut sont également appliquées et relues, puis les valeurs
initiales restaurées. Les paramètres initiaux confirmés sont respectivement
`120, 14, 16, 0, 12, 92, 128, 2`.

Le premier essai de réglages s'est terminé au même moment que la validation du
bouton et a été interrompu par sa fermeture automatique ; il n'est pas compté
comme une réussite. Les valeurs ont été explicitement rétablies et l'essai
séparé a ensuite entièrement réussi.

Deux validations automatiques successives passent : quatre PNG de 1280 × 1024,
AVI de 32 images lisibles à 8 fps, noms/date/heure/yeux/dossiers corrects,
sauvegarde/restauration de cinq captures avec médias identiques. Elles vérifient
également l'arrêt du lecteur et la disparition de ses sockets privées à chaque
fermeture. Après le retour au pilote Apple, le chemin `--avfoundation` passe
à son tour la validation automatique de captures de pleine résolution.

## Défauts corrigés pendant les essais

La normalisation Foundation transforme un chemin existant `/private/tmp/...`
en `/tmp/...`. Le lanceur rejetait donc une réponse USB valide. La vérification
est désormais lexicale et restreinte au chemin exact créé par le service ;
un test couvre le chemin existant et les tentatives de traversée.

La fermeture automatique révélait une terminaison `SIGPIPE` du lecteur avant
le nettoyage final. Le lecteur traite désormais les ruptures de communication
comme des erreurs d'I/O, y compris celles des tubes internes de libusb. Le
service laisse aussi une courte période de fermeture normale avant d'envoyer
un signal. Les essais finaux se terminent avec `USB_HELPER_DONE`, sortie normale
`reason=1 status=0`, arrêt du lecteur et suppression de ses sockets.

## Signature, paquets et tests de refus

Le ZIP et le DMG reçoivent une signature ad hoc gratuite, vérifiée avec
`codesign --verify --deep --strict`. Leurs 47 fichiers sont identiques ; leurs
empreintes SHA-256 sont vérifiées. L'application installée provient du DMG.

Le contrôle des commandes de chargement Mach-O a montré que le lecteur USB
exige macOS 15. Le minimum du paquet est donc **15.0** ; l'API XPC utilisée par
le service est disponible dès 13 mais ne détermine pas à elle seule le minimum
du paquet complet. Les compilations USB définissent explicitement la cible 15.0.
La correction finale de cette métadonnée n'a changé ni le lecteur, ni le
service, ni le GUI, ni la section de code du lanceur validés avec le bouton.
Le paquet corrigé est ensuite réinstallé et passe de nouveau les captures
automatiques sans authentification par ouverture.

Un programme signé avec le même identifiant d'application mais un autre CDHash
est refusé par XPC et ne peut demander une réception privilégiée. Un paquet
modifié est refusé par l'installateur avant toute modification des fichiers
protégés. Deux installations concurrentes sont bloquées avant toute mutation.
Les tests natifs refusent également liens symboliques, FIFOs, dossiers, fichiers
trop volumineux et manifestes possédés par un utilisateur ordinaire.

Les 13 tests Python sont exécutés : 12 passent, un test Debian est ignoré sur
Mac faute de `dpkg-deb`. Les petits composants natifs compilent avec `-O2`,
`-Wall -Wextra -Werror`. `cargo fmt --all --check` et `git diff --check` passent.
Aucune compilation Rust ou reconstruction des bibliothèques USB n'est effectuée
pendant ce développement ; seule la compilation des petits composants natifs
est nécessaire. Le GUI release reste identique à celui validé le 7 octobre :

```text
SHA-256 IrisScopeGui:
169a2eee32b22c46725c6a8ddc9bb40a60e2f43209bfabd480eb9720c1fc8ba6
```

Les paquets contiennent les sources/licences et l'objet permettant de relier
le lecteur avec une bibliothèque libusb modifiée. Aucun SDK propriétaire,
certificat payant, modification de SIP ou de `sudoers` n'est utilisé.

## Limites de cette validation

L'essai matériel porte sur ce Mac Intel et ce DE400. Windows, Apple Silicon,
macOS antérieurs à 15, sessions physiques d'une journée, débranchement/rebranchement
et veille/réveil ne sont pas validés ici. La réception normale n'a pas de limite
horaire ; les délais des commandes de diagnostic ne limitent pas l'utilisation
ordinaire. MP4 exige FFmpeg optionnel et n'est pas validé dans cet essai ; les
AVI originaux sont conservés. Les contrôles déjà exécutés sur le logiciel Rust
sont documentés dans le rapport précédent et ne sont pas présentés comme
recompilés aujourd'hui.
