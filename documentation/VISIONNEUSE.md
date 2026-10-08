# Visionneuse photo

La visionneuse sert à examiner les captures, comparer des prises de vue et conserver des observations manuelles. Les références d’iridologie restent des aides visuelles, sans diagnostic ni interprétation automatique.

## Examiner une photo

La barre principale conserve la navigation, le zoom, **Exporter…** et **Fermer**.
**Outils** réunit les annotations, la comparaison, les références, les réglages
d’affichage et la marque de photo retenue. **Vue** réunit la taille réelle, la
loupe, l’image seule, le plein écran et la présentation. Ces menus suivent le
thème choisi dans les paramètres.

- **Ajuster** : photo entière ; un double-clic produit le même résultat.
- **Vue → Taille réelle · 100 %** : taille réelle, un pixel de la photo pour un pixel physique de l’écran, y compris avec une échelle de bureau agrandie.
- **Molette**, **−/+**, **curseur** : zoom de 10 à 800 %. Le curseur met à jour l'image pendant le glissement, par pas de 1 %, sans attendre le relâchement. La molette conserve la zone sous le pointeur, sauf lorsqu’une limite de déplacement est atteinte.
- **Glisser** ou **flèches** : déplacement dans l’image. **Vue → Loupe** : agrandissement local ×2,5 du cadrage affiché, sans créer de détail supplémentaire.
- **Vue → Plein écran** : agrandir la fenêtre. Échap quitte le plein écran avant de fermer la photo.
- **Vue → Image seule** : libérer l’espace des barres d’outils et du panneau latéral. Les commandes se masquent après trois secondes sans interaction. Un mouvement dans l’image ou le bouton **Commandes** les fait réapparaître. Échap rétablit d’abord la vue complète, puis quitte le plein écran, puis ferme la photo.
- **Vue → Présenter** : montrer l’iris en masquant noms, numéros de dossier, légendes de fichiers, notes et annotations. L’œil reste indiqué. **Fin présentation** rétablit l’accès aux informations privées ; Échap fait de même depuis la caméra, après la fermeture des autres vues. Ce mode masque l’affichage local ; les exports et fichiers existants conservent leurs données.

Les images du panneau **Références** et de la bibliothèque utilisent aussi la molette, le glissement et les flèches. Leur zoom va de 100 à 400 %, avec un curseur synchronisé même après une modification à la molette. **Ajuster** recentre l’image complète.

Le réglage **Rotation** ouvre ses commandes à côté du panneau photo ; dans le direct, elles s’ouvrent au-dessus de la barre d’outils. La fenêtre reste dans les limites de l’application.

## Notes et annotations

**Outils → Annoter et noter** propose un menu pour choisir Déplacer, Cercle, Flèche, Point ou Texte. Pour le cercle, glisser du centre au bord ; pour la flèche, glisser du départ à la pointe. Pour le texte, saisir le libellé puis cliquer sur l’image. Les cercles et flèches montrent un aperçu pendant le tracé. **Déplacer** revient à la navigation dans l’image.

Les annotations peuvent être masquées, retirées individuellement ou annulées. L’annulation conserve jusqu’à 32 modifications de la session de la photo ; elle ne modifie pas les notes. Les observations sont enregistrées après une pause de 700 ms ou par **Enregistrer**. La limite est de 32 000 octets de notes, 256 annotations et 512 octets par libellé.

La navigation et la fermeture sauvegardent les modifications en attente. Un échec conserve le brouillon et arrête les tentatives automatiques jusqu’à une nouvelle modification ou un enregistrement manuel. On peut copier les notes et utiliser **Fermer sans enregistrer** pour abandonner explicitement le brouillon. Si la lecture des observations échoue à l’ouverture, leur édition est bloquée pour protéger les données existantes ; rouvrir la photo permet de réessayer.

L’état d’enregistrement reste affiché : modifications en attente, enregistrement en cours, notes enregistrées avec leur heure, ou échec. **Outils → Retenir cette photo** marque manuellement une prise à conserver pour l’analyse ; **Photo retenue** permet de retirer cette marque. Elle est enregistrée avec les observations et rappelée dans les miniatures du dossier et du choix de comparaison.

L’original n’est jamais réécrit. Les observations et leur date de modification sont dans `.iriscope-index.json`, avec une empreinte SHA256 du contenu original. Elles font partie des sauvegardes, se retrouvent après restauration et ne sont pas appliquées à un fichier remplacé. Ce stockage reste local, non chiffré, comme le reste de la bibliothèque.

## Comparaison et références

**Outils → Comparer les prises** permet de rechercher les prises du même dossier et du même œil, par période facultative. **Filtrer par date** déplie les champs **Du / Au**, qui utilisent JJ/MM/AAAA et proposent un calendrier ; AAAA-MM-JJ reste accepté. Jusqu’à 200 photos récentes sont proposées avec miniature, date, œil et marque **Retenue** ; préciser une période permet de retrouver une série plus ancienne. La sélection n’altère pas les filtres de la bibliothèque. **Utiliser comme référence** conserve la photo à gauche ; ouvrir ensuite une autre prise permet de les comparer.

Dissocier les vues permet de régler leur cadrage séparément. Relier zoom et déplacements conserve l’alignement obtenu. Le zoom indiqué sous chaque photo permet de connaître les échelles utilisées. Les légendes rappellent la date, l’œil et le dossier. Une comparaison visuelle dépend du cadrage et des conditions d’éclairage des prises de vue.

**Outils → Références visuelles** affiche les images importées, avec zoom et déplacement. Les trois cercles repères sont facultatifs ; cliquer **Placer le centre**, puis sur la pupille, et ajuster leur rayon. Aucun organe, signe ou maladie n’est identifié automatiquement.

## Affichage et exports

Luminosité, contraste, rotation et miroir modifient seulement l’affichage. **Original** les suspend sans effacer les réglages ; **Réinitialiser l’affichage** les remet à zéro. Chaque nouvelle ouverture commence avec l’original.

**Exporter…**, en haut de la visionneuse, ouvre les deux commandes de copie :

- **Copie PNG annotée…** : copie en pleine résolution, avec les réglages d’affichage actifs et toutes les annotations, même masquées à l’écran. La loupe, le zoom de navigation et les cercles repères ne sont pas exportés.
- **Fiche PDF et notes…** : original annoté, légende, notes et, si active, photo de comparaison. Les réglages d’affichage sont exclus. Les notes longues continuent sur plusieurs pages.

Les exports utilisent les observations affichées au lancement, conservent les originaux et refusent d’écraser un fichier existant. **Annuler** retire la copie temporaire avant sa publication ; la fermeture de l’application attend le nettoyage. Les sélecteurs de fichiers sont ceux du système.

## Dossiers et notes générales

**Dossiers / Séances**, dans la bibliothèque, recherche par nom ou numéro stable. Choisir le dossier, puis la date, affiche ses photos gauche/droite, leurs miniatures et les prises retenues. La journée actuelle est proposée même sans capture. Jusqu’à 200 photos récentes sont affichées par journée ; les vidéos et les captures sans œil renseigné restent dans la bibliothèque. **Dossier → Historique des séances**, dans la zone Patient, ouvre le dossier actif. **Utiliser pour la capture** reprend le dossier sélectionné pour les nouvelles prises.

Les **Notes générales** concernent ce dossier et cette journée. Elles sont distinctes des notes de chaque photo et ne sont pas ajoutées à la fiche PDF d’une photo. Limitées à 32 000 octets, elles se sauvegardent après une pause de 700 ms ou par **Enregistrer**. Leur état et l’heure du dernier enregistrement restent visibles. La fermeture et le changement de journée attendent la sauvegarde ; changer de dossier est possible après l’enregistrement. Un échec conserve le brouillon et propose un abandon explicite.

Les journées sont des regroupements de captures et de notes, sans agenda. Le stockage local dans `.iriscope-index.json` est inclus dans les sauvegardes/restaurations et reste non chiffré. Le compteur gauche/droite de la caméra concerne seulement la session de capture en cours, avec changement d’œil manuel.

## Adapter l’interface

Dans **Paramètres → Interface et affichage**, choisir le thème, la mémorisation de la disposition, l’ouverture des photos en **Image seule** et les miniatures **Petites / Moyennes / Grandes**, ou une largeur de 80 à 480 pixels avec le curseur, par pas d'un pixel logique. La taille se règle aussi depuis la bibliothèque et s’applique à la grille comme à la liste.

Les états d'enregistrement des observations partagent une ligne de hauteur fixe. Une modification, une sauvegarde ou un long message d'erreur ne décale plus la photo. Le détail d'un message tronqué est disponible au survol.

Les séparateurs des panneaux patient, photo et dossiers se glissent horizontalement. Ils limitent la largeur pour conserver une zone d’image utilisable. Au clavier, les flèches ajustent la largeur ; un double-clic rétablit la largeur adaptative. Le panneau patient se masque sur place et le panneau photo se ferme avec **×**. Les champs du patient et les sélecteurs de capture restent fixes ; seule la zone de dernière capture défile si elle manque de place. La grille de bibliothèque reste alignée à gauche lorsque la taille des vignettes change.

La mémorisation retrouve les panneaux, leurs largeurs et la vue grille/liste, sans rouvrir un dossier, une photo ou un plein écran. Quand elle est désactivée, ces éléments retrouvent leur disposition par défaut au prochain lancement ; les choix explicites de taille de miniature, présentation et image seule restent conservés. **Réinitialiser la disposition** rétablit les options d’interface par défaut sans changer le thème, les dossiers de capture, les notes ou les originaux.

Le **Diagnostic caméra** conserve l’état de connexion visible. **Détails** déplie les informations techniques, repliées par défaut. L’aide et les raccourcis existants restent disponibles.
