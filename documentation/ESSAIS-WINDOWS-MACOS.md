# Essayer Iriscope sur Windows et macOS

Le code se développe depuis Linux. Ces machines servent à vérifier les pilotes,
les autorisations, l’installation et l’iriscope réel ; Rust et Python ne sont pas
nécessaires pour ces essais.

## Choisir et installer le paquet

Sur GitHub, ouvrir [**Actions → CI**](https://github.com/Epikaigle/iriscope-app/actions/workflows/ci.yml?query=branch%3Amain),
puis la dernière **validation complète réussie** de `main` qui contient la
rubrique **Artifacts**. Les CI automatiques légères ne produisent pas
d’installateur. Les paquets déjà vérifiés sont aussi disponibles dans
[la validation complète du 6 octobre 2026](https://github.com/Epikaigle/iriscope-app/actions/runs/37529803272).

Pour générer des paquets du code actuel, sélectionner **Actions → CI → Run
workflow**, choisir `main` et laisser **full_validation** cochée. Attendre la
réussite des quatre plateformes natives. Une connexion à GitHub est nécessaire
pour télécharger les artefacts dans **Artifacts**, en bas de l’exécution.
Télécharger celui de la machine concernée et extraire son ZIP :

| Machine | Artefact | Installation |
| --- | --- | --- |
| Windows 64 bits | `iriscope-Windows-X64` | Lancer `IrisScope-0.4.0-Setup.exe` |
| Mac avec puce Apple M1/M2/M3/M4 ou suivante | `iriscope-macOS-ARM64` | Ouvrir le DMG, glisser `IrisScope.app` dans Applications |
| Mac Intel | `iriscope-macOS-X64` | Ouvrir le DMG, glisser `IrisScope.app` dans Applications |

Sur Mac, **menu Apple → À propos de ce Mac** indique la puce ou le processeur.
Les noms techniques des exécutables et du bundle restent `IrisScope` pour
préserver la mise à jour des installations existantes ; l’application s’appelle
Iriscope. Les deux paquets Mac ont le même nom de DMG ; choisir d’abord le bon artefact
**ARM64** ou **X64**. Les archives portables sont également présentes.

Les paquets d’essai ne sont pas signés. Sur macOS, tenter une première ouverture
depuis Applications. Si le développeur n’est pas reconnu, ouvrir **Réglages
Système → Confidentialité et sécurité**, puis **Ouvrir quand même** pour cette
application : c’est la [procédure décrite par Apple](https://support.apple.com/fr-fr/102445).
Autoriser ensuite la caméra lorsque le système le demande. Les captures et
paramètres restent hors du programme.

## Essai normal

Fermer les autres logiciels utilisant la caméra, brancher l’iriscope, puis ouvrir
Iriscope. Avec un dossier fictif, vérifier le direct, une photo de chaque œil et
une vidéo. Tester le bouton physique en Photo, puis en Vidéo pour démarrer et arrêter.
Un appui prolongé doit produire une seule action.

Vérifier aussi la bibliothèque, les annotations, le zoom à la molette et au curseur,
la rotation, les calendriers et le retour de Présenter avec le panneau masqué.
Les contrôles caméra proposés dépendent du pilote ; macOS en expose moins.

## Diagnostic et essai guidé du bouton

Fermer Iriscope avant d’exécuter ces commandes : elles ouvrent elles-mêmes la caméra.
Le chemin d’essai doit être inexistant ou vide. Il contient uniquement des dossiers
fictifs et conserve les résultats pour inspection ; les paramètres habituels restent intacts.

### Windows — PowerShell

```powershell
& "$env:LOCALAPPDATA\Programs\IrisScope\IrisScope.exe" --diagnose
& "$env:LOCALAPPDATA\Programs\IrisScope\IrisScope.exe" --validate-button "$env:USERPROFILE\Desktop\IrisScope-test-bouton"
```

La réception Windows tente un abonnement sur la source Media Foundation ouverte.
Si le pilote exige une cible KS distincte (pin ou nœud), cet abonnement peut être
refusé : le récepteur actuel ne découvre pas cette autre route. Conserver l’erreur
exacte du diagnostic pour poursuivre le port depuis Linux. Les captures à l’écran
restent disponibles.

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
