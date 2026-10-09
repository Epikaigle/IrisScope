# Mises à jour d’IrisScope

Depuis la version 0.4.1, **Paramètres → Rechercher une mise à jour** permet
de télécharger une nouvelle version sans ouvrir GitHub. Une vérification se
fait aussi dix secondes après le démarrage. Seules les releases stables et
les paquets du système et de l’architecture utilisés sont proposés.

1. **Télécharger** prépare la nouvelle version et vérifie sa signature.
2. **Installer et redémarrer** ferme proprement IrisScope, installe puis relance.
   Terminer les enregistrements et laisser les notes se sauvegarder auparavant.

Une connexion absente ou un téléchargement annulé ne bloque pas la caméra.
Les captures, les dossiers et les réglages ne sont pas remplacés. Sur Mac,
l’installation doit se trouver dans un dossier où le compte peut écrire,
par exemple `~/Applications`. Une authentification peut être nécessaire pour
approuver le nouveau paquet USB. Elle n’est pas demandée à chaque ouverture.
Le paquet reste signé ad hoc gratuitement, sans notarisation Apple.

Sur Linux, le `.deb` utilise `pkexec` et `dpkg` pour installer après la demande
d’authentification du système. L’archive portable peut se mettre à jour sans
administrateur dans un dossier personnel ; elle nécessite `curl`. Windows
et Apple Silicon ne sont pas proposés par cette première release.

Le manifeste et chaque fichier sont vérifiés avec la clé publique intégrée.
Une signature invalide, un fichier tronqué, un autre système, un autre tag ou
une version plus ancienne ne peut pas devenir une mise à jour. Les archives
refusent les liens et les chemins sortant du dossier temporaire.

## Publier une nouvelle version

Les tests sont exécutés localement. Le seul workflow du dépôt construit les
paquets macOS Intel et Linux, les signe et les publie après un tag `vX.Y.Z`.
La version du tag doit correspondre à `Cargo.toml`. Une release incomplète
reste en brouillon et n’est pas proposée aux installations existantes.

La clé privée de mise à jour est gratuite et distincte d’un certificat Apple.
Elle reste hors du dépôt, dans un dossier protégé du poste de développement,
et dans le secret GitHub `IRISCOPE_UPDATE_PRIVATE_KEY`. La clé publique seule
est dans `crates/iriscope-updater/public-key.hex`. Conserver cette clé privée
pour les versions suivantes ; changer la clé nécessite une transition explicite.

Les paquets Linux ciblent Ubuntu 24.04 ou une distribution compatible avec ses
bibliothèques. Les essais logiciels sur Mac ne remplacent pas un essai réel
de l’authentification et de l’installation Debian sur Linux.
