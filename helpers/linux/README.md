# Bouton physique du DE400 sous Linux

Le DE400 Infoxelle `21cd:603b` testé transmet `02 01 00 01` à la pression et
`02 01 00 00` au relâchement sur son endpoint d’interruption `0x81`.
Son descripteur annonce `bTriggerUsage = 0` : le pilote `uvcvideo` ne crée donc
pas de périphérique clavier pour ce bouton. Les métadonnées vidéo ne contiennent
pas de marqueur STILL_IMAGE pendant les pressions constatées.

La passerelle lit ces événements via `usbmon` et transmet uniquement pression
et relâchement à IrisScope, sur un socket local associé au port USB de la caméra.
Elle ne capture pas d’images, ne modifie aucun contrôle USB et ne détache pas le
pilote. Elle n’exige pas de lancer IrisScope en administrateur. Les connexions
sont réservées au groupe système `video`; le socket et le programme installé
appartiennent à root. Les doublons d’une pression maintenue sont ignorés.

Les captures suivent les réglages de l’application : Photo, Vidéo ou l’action
fixe choisie dans les paramètres. En mode Vidéo, une pression démarre et la
suivante arrête l’enregistrement. Sans passerelle, la caméra et le bouton logiciel
fonctionnent normalement.

## Installer

Depuis la racine du dépôt :

```sh
sudo bash scripts/install-de400-button.sh
```

Fichiers installés :

- `/usr/local/lib/iriscope/de400_button_bridge.py` : programme appartenant à root ;
- `/etc/systemd/system/iriscope-button.service` : service local ;
- `/etc/modules-load.d/iriscope-button.conf` : chargement de `usbmon` au démarrage ;
- `/run/iriscope-button/status.sock` : socket temporaire, recréé par le service.

Le service démarre immédiatement et aux prochains démarrages du PC. `debugfs`
doit être disponible à `/sys/kernel/debug`. Le compte qui exécute IrisScope doit
appartenir au groupe `video`. Cette intégration concerne Linux ; Windows et
macOS nécessitent encore leurs essais matériels.

Python 3, `systemd` et `modprobe` doivent être disponibles. Dans l'archive
portable, ces mêmes chemins sont présents à côté de l'exécutable. Avec le paquet
Debian, utiliser `sudo bash /usr/share/doc/iriscope/scripts/install-de400-button.sh`.
L'installation de l'application ne lance pas automatiquement cette installation
système facultative.

Vérification : `systemctl status iriscope-button.service` et
`journalctl -u iriscope-button.service`. Les journaux ne contiennent pas d’images
ni de noms de patients.

## Retirer

```sh
sudo bash scripts/uninstall-de400-button.sh
```

Cette commande retire uniquement la passerelle et ses fichiers système, conserve
les captures et les paramètres, et laisse le module partagé `usbmon` chargé
jusqu’au prochain redémarrage.

## Diagnostic sans installation

Fermer le direct, puis lancer `sudo python3 scripts/diagnose-de400-usb-status.py`.
Effectuer trois pressions espacées après `STATUS_ARMED`. L’observation se termine
au bout de 45 secondes sans enregistrer d’images.

Source du comportement du pilote :
[Linux 6.8, uvc_status.c](https://github.com/torvalds/linux/blob/v6.8/drivers/media/usb/uvc/uvc_status.c).
La lecture vide la file par lots car l'interface texte de
[usbmon](https://github.com/torvalds/linux/blob/v6.8/drivers/usb/mon/mon_text.c)
retourne un événement par appel, même avec un grand tampon de lecture.
