# Essayer IrisScope sur Windows et macOS

Le code se développe depuis Linux. Ces machines servent à vérifier les pilotes,
les autorisations, l’installation et l’iriscope réel ; Rust et Python ne sont pas
nécessaires pour ces essais.

## Choisir et installer le paquet

Sur GitHub, ouvrir **Actions → CI**, puis le dernier passage réussi de la branche
contenant les corrections. Télécharger l’artefact correspondant et extraire son ZIP :

| Machine | Artefact | Installation |
| --- | --- | --- |
| Windows 64 bits | `iriscope-Windows-X64` | Lancer `IrisScope-0.4.0-Setup.exe` |
| Mac avec puce Apple M1/M2/M3/M4 ou suivante | `iriscope-macOS-ARM64` | Ouvrir le DMG, glisser IrisScope dans Applications |
| Mac Intel | `iriscope-macOS-X64` | Ouvrir le DMG, glisser IrisScope dans Applications |

Sur Mac, **menu Apple → À propos de ce Mac** indique la puce ou le processeur.
Les archives portables sont également présentes. L’application est personnelle et
non signée : le système peut demander une ouverture explicite. Autoriser la caméra
lorsque le système le demande. Les captures et paramètres restent hors du programme.

## Essai normal

Fermer les autres logiciels utilisant la caméra, brancher l’iriscope, puis ouvrir
IrisScope. Avec un dossier fictif, vérifier le direct, une photo de chaque œil et
une vidéo. Tester le bouton physique en Photo, puis en Vidéo pour démarrer et arrêter.
Un appui prolongé doit produire une seule action.

Vérifier aussi la bibliothèque, les annotations, le zoom à la molette et au curseur,
la rotation, les calendriers et le retour de Présenter avec le panneau masqué.
Les contrôles caméra proposés dépendent du pilote ; macOS en expose moins.

## Diagnostic et essai guidé du bouton

Fermer IrisScope avant d’exécuter ces commandes : elles ouvrent elles-mêmes la caméra.
Le chemin d’essai doit être inexistant ou vide. Il contient uniquement des dossiers
fictifs et conserve les résultats pour inspection ; les paramètres habituels restent intacts.

### Windows — PowerShell

```powershell
& "$env:LOCALAPPDATA\Programs\IrisScope\IrisScope.exe" --diagnose
& "$env:LOCALAPPDATA\Programs\IrisScope\IrisScope.exe" --validate-button "$env:USERPROFILE\Desktop\IrisScope-test-bouton"
```

### macOS — Terminal

```bash
/Applications/IrisScope.app/Contents/MacOS/IrisScope --diagnose
/Applications/IrisScope.app/Contents/MacOS/IrisScope --validate-button "$HOME/Desktop/IrisScope-test-bouton"
```

Suivre les messages du terminal : deux pressions pour deux photos, une troisième
pour démarrer la vidéo, attendre au moins huit secondes, puis une quatrième pour
l’arrêter. Le rapport final est enregistré dans `validation-report.txt` du dossier
d’essai. Il vérifie le classement, les noms, les médias et la sauvegarde/restauration.
L’absence de FFmpeg indique **NOT TESTED** pour le MP4 ; elle n’empêche pas la
validation des captures et de la sauvegarde. Une autre erreur d’export reste un échec.

« Abonnement natif Windows actif » ou « réception USB macOS active » signifie que
le récepteur a été ouvert. Seules les pressions réelles confirment le fonctionnement.
En cas de refus du pilote, conserver le message exact. Pour vérifier séparément la
caméra et les captures logicielles, remplacer `--validate-button` par
`--validate-hardware` avec un **autre dossier vide**.

## Stabilité et retour d’essai

Faire aussi un direct de 30 minutes, une déconnexion/reconnexion et un redémarrage
du PC ou du Mac. Le [protocole complet](VALIDATION-MATERIELLE.md) détaille les autres
parcours de vérification.

Pour chaque problème, noter la version, le système, le modèle de Mac ou de PC,
le message du diagnostic et les étapes exactes. Indiquer les essais réussis,
échoués et non réalisés. Un retour permet de corriger le code depuis Linux.
