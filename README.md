# AWorld

Bac à sable 2D en Rust (macroquad pour le rendu GPU avec shaders, rapier2d pour les solides) :
on construit un monde, on lance la physique, puis des voitures le traversent (entrée → sortie, façon Lemmings).
Tout est pensé pour apprendre en jouant : température, eau, roches, métaux, plantes, saisons.

```
cargo run --release
```

## Jouer
- **Panneau de gauche** (onglets) : Matières, Blocs, Voitures, Outils, Monde. Survole une case pour lire l'explication.
- **Palette de droite** (Vues) : elle sort quand la souris va au bord droit. Normale, Température (°C de chaque cellule),
  Efforts (charge des pièces, vert → rouge), Vent.
- **Barre du haut** : Jouer/Pause, Retour, Annuler (plusieurs fois, même en simulation), Voiture, vitesse, Tourner,
  emplacements 1-2-3, Sauver, Charger, Monde, Vide, Science (toutes les transformations), Plein écran.
- **Construction** : le monde est figé tant qu'on n'a pas lancé. Pause redonne la main pour un 2e tour d'édition.

## Le monde
- **Décors** (Monde → Décor) : campagne, montagne (cimes enneigées), mer. Chacun a son arrière-plan.
- **Jour et nuit**, soleil (la chaleur forme un gradient autour de lui), lune, étoiles ; **saisons** : les feuilles
  rougissent, tombent, pourrissent en terre ; les arbres repoussent au printemps.
- **Température** : l'air refroidit avec l'altitude ; l'eau bout à 100 °C, gèle sous -6 °C ; la roche fond vers 540 °C ;
  la lave refroidit très lentement ; le sable chauffé devient verre (et casse sous un choc thermique).
- **Cycle de l'eau** : évaporation → vapeur → nuage (que le vent pousse) → pluie, neige ou grêle selon le froid.
  Un nuage réchauffé par le soleil garde son eau. Les gaz qui sortent d'un côté reviennent de l'autre.
- **Chimie** : bois + feu = cendre (carbone) ; fer + cendre + chaleur = acier ; l'eau rouille le fer ; rouille + feu = fer ;
  eau + lave = pierre ; roche + gel = sable ; verre brisé = éclats puis sable.
- **Plantes** : terre humide + graine + eau + lumière (jamais la nuit) = arbre ; les arbres font des graines : une forêt pousse.
  La terre nue s'érode sous l'eau qui coule (l'herbe la protège). Une graine sans eau pourrit en terre.
- **Pas de vrai sol** : seule la roche posée sur le bas du monde retient le reste ; tout ce qui n'y est pas relié tombe et
  disparaît par le fond. Sable, eau et gaz sortent aussi par les côtés (sauf les gaz, recyclés de l'autre côté) ; le ciel
  bloque les gaz. On peut donc creuser le fond pour vider un lac.

## Construire
- **Blocs façon Lego** : brique, longue, planche, poutre fer, poutre acier, cube, verre, roue. Aimantés sur la grille de plots
  et sur leurs voisins (Maj = libre), pas de superposition, molette ou Q/E pour tourner. Les blocs qui se touchent sont soudés ;
  bout à bout, les tiges de même matière forment une seule pièce plus rigide.
- **Tiges** (bois, fer, acier) : un clic fixe un bout (sol, bloc ou nœud), tire, un second clic fixe l'autre. Longueur max 110.
  Elles pivotent aux nœuds : on dessine des fermes de pont. Le bois est léger et fragile, le fer lourd, l'acier lourd et solide.
- **Câble** (acier, fer, corde) : deux points — bloc, tige (bout ou milieu), sol ou n'importe quel bord du monde, ciel compris (les tiges, ressorts, élastiques et câbles s'y accrochent). Élastique : il prend la forme
  d'une parabole sous son poids. Le curseur d'élasticité le rend plus ou moins raide ; l'acier tient le plus, la corde s'étire.
  Une tige posée sans attache tombe.
- **Matériaux** : un bloc consomme des cellules de la matière dont il est fait (brique = terre, planche = bois, poutre = fer...).
  En édition on peut choisir « illimité » ; en simulation c'est toujours limité.
- **Marteau** : casse le verre en 3 étapes (bloc → moitiés → éclats → sable) et la glace. Un choc, une chute ou une voiture aussi.
- Le **bois flotte** (une voiture peut rouler dessus, mais ça bouge), le fer et la pierre coulent.

## Voitures
Citadine, Course, 4x4, Camion : suspensions, couple moteur, adhérence. Elles sautent, font demi-tour devant un obstacle, passent
à travers les arbres, brisent le verre, explosent au choc trop fort (contre un mur ou une autre voiture), sont perdues dans le
vide ou en sortant du monde. Leur poids pèse sur les ponts. **Réglages** (onglet Voitures) : puissance, vitesse, suspension,
amortisseurs, adhérence, taille des roues, solidité, poids, couleur — sur la voiture choisie ou pour les suivantes.

## Raccourcis
Espace jouer/pause · R retour · Z annuler · C voiture · G monde suivant · N vide · H température · X efforts · V vent ·
Q/E/T tourner · Tab onglet · [ ] pinceau · ↑/↓ vitesse · 1-0 matières · F5/F9 sauver/charger · F11 plein écran.

## Développement
`cargo test` (≈70 tests : simulation, voitures, blocs, sauvegarde). `cargo run --release -- --demo out.png [--tab cars]
[--biome mer|montagne] [--hour 18.5] [--season hiver] [--view thermique|efforts|vent] [--palette]` enregistre une capture.


## Son

Tout le son est synthétisé au démarrage (aucun fichier audio) : moteur (4 régimes mélangés selon la vitesse),
vent fort, eau qui coule, feu/lave qui crépitent, explosions, chocs et objets qui tombent, verre qui casse,
tiges/câbles/ressorts qui cèdent, sable projeté. **M** coupe/remet le son ; `--mute` démarre sans son.
Sans carte son (ALSA/PipeWire), compiler avec `cargo build --release --no-default-features`.

## Sol meuble et vent

- Sable, boue, neige, terre et herbe sont meubles pour les voitures : un peu de traînée, de l'adhérence en moins,
  du sol projeté derrière les roues, et une voiture lente qui patine peut s'enliser (elle se dégage dès qu'elle repart).
- Le vent pousse aussi les blocs libres (surface × vent²), et les arbres se penchent et ondulent avec lui
  (effet visuel, plus fort en haut).
- Câbles, ressorts et tiges : le point d'ancrage s'affiche (point vert) et la souris colle au point ; près d'un
  bord du monde, ce bord s'illumine.
- Les liquides (eau, huile, lave) traversent les arbres au lieu d'être retenus.

## Compiler, lancer, installer

| | Linux | Windows |
|---|---|---|
| Compiler l'application | `./build.sh` | `build.bat` |
| Compiler la version web (wasm + petit serveur) | `./build.sh web` | `build.bat web` |
| Compiler **et lancer** l'application | `./run.sh` | `run.bat` |
| Compiler **et servir** la version web | `./run.sh web` | `run.bat web` |
| Installer + raccourci bureau et menu | `./install.sh` (`--uninstall` pour retirer) | `install.bat` (`install.bat uninstall`) |

- `run` accepte les options du jeu (`./run.sh --mute`) ; `run web [port]` ouvre http://localhost:8765/.
- Le paquet web est dans `dist/web/` (page + wasm + `aworld-serve`) ; `dist/web/site/` peut aussi être posé sur
  n'importe quel hébergeur statique. Il faut `rustup` (le script installe la cible `wasm32-unknown-unknown`).
- Version web : les sauvegardes restent en mémoire tant que la page est ouverte (pas de fichiers).
- `install` copie le jeu dans `~/.local/share/aworld` (Linux) ou `%LOCALAPPDATA%\AWorld` (Windows), où vivent aussi
  les mondes sauvegardés. Sous GNOME, si le raccourci du bureau reste grisé : clic droit, « Autoriser le lancement ».
  Les scripts Windows n'ont pas pu être essayés depuis Linux.

## Raccourcis

Espace jouer/pause · R retour · C voiture · Z annuler · H/X/V vues · M son · F ou F11 plein écran · Tab onglet. Poteau (brique haute) dans les blocs : plus besoin de tourner les blocs.
**Échap** recule d'un cran : page Science, tracé en cours, plein écran, puis une fenêtre « Quitter AWorld ? » qui met
la simulation en pause ; Échap y reprend le jeu, **Entrée** quitte.

## Mécanique (onglet Méca)

Pièces : **moteur** (petit engrenage vu de face), **engrenages** petit/moyen/grand, **axe** (relie deux engrenages
éloignés, ou un engrenage à une roue), **coulisseau** (un rail et son chariot), **bielle** (d'un point d'accroche
d'engrenage au chariot) et **pivot** (une roue sur un axe : poulie, tambour de treuil, roue motrice).

- Les engrenages s'engrènent quand leurs centres sont à `r1 + r2` ; la pose s'aimante à cette distance (en ligne
  droite ou en diagonale), sur le centre d'un bloc, ou s'aligne sur un autre engrenage. Les rails se redressent
  (horizontal, vertical, diagonale) et leurs bouts s'accrochent au sol, aux blocs, aux tiges. Les points d'accroche
  des bielles sont fixes sur l'engrenage (deux couronnes et le centre) et s'allument au survol.
- Le calcul est dynamique : inertie, courbe couple/vitesse du moteur, rapports exacts. Un engrenage trop loin ne
  s'engrène plus et tourne libre ; un coulisseau bute sur la roche ou le bord de son rail et le moteur cale ; une
  bielle trop sollicitée casse, des dents trop sollicitées lâchent ; un nombre impair d'engrenages en cercle se
  coince. Un volant d'inertie (grand engrenage) passe les points morts.
- Option moteur : **sens inverse si surcharge** (cale, butée ou charge lourde). Clic sur un moteur posé : inverser son sens.
- Le chariot pousse sable, eau et blocs. Un engrenage relié à une roue pivotée l'entraîne ; un câble accroché sur le bord
  de cette roue (le câble s'accroche au bord des roues) s'enroule : c'est un **treuil**.
- **Treuil** (tambour à enrouleur sur un engrenage) : avec l'outil Câble, un clic sur le tambour y fixe un bout de câble ;
  en tournant, le tambour enroule ou déroule (sens positif = enroule) et le câble tire en retour sur le moteur. Le
  tambour a une butée en fin de câble (le moteur cale, ou repart dans l'autre sens s'il est réglé ainsi).
- **Poulie** (une poulie sans roue, posée sur un bloc, le sol ou dans le vide) : outil Câble : premier bout, puis la
  poulie, puis l'autre bout (plusieurs poulies à la suite possibles). Le câble glisse dessus sans frottement
  (machine d'Atwood, palan). Une roue reste libre ; l'outil Pivot en fait un axe si on le souhaite.
- Une bielle s'accroche à tout : engrenage, chariot, bloc, tige, sol. Sur un bloc, la bielle le tire ou le pousse
  vraiment (l'effort revient sur le moteur). Un coulisseau accepte plusieurs bielles, et une bielle peut relier
  deux engrenages.
- Télécommande du moteur (sélectionne un moteur : sous les Vues) : en marche/arrêt, sens, vitesse à vide, couple,
  seuil de retournement, et les mesures en direct (tr/min, charge). Vue Efforts : couple, charge des dents, force
  des bielles, pression sur les chariots.
- Câbles : un bout peut rester libre (le second clic dans le vide) ; l'acier est bien plus raide.

## L'air qui s'écoule

Le vent est un écoulement simplifié (grille grossière, fluide incompressible) : des couches d'air soufflent en sens
opposés selon l'altitude, des rafales s'enroulent en tourbillons, l'air chaud monte au-dessus d'un feu, et
l'écoulement contourne le relief. Nuages, vapeur et fumée le suivent, la pluie se penche avec le vent local.
Touche **V** pour voir les flèches.

## Fluides, pression, outils

- **Onglet Fluides** : tuyaux (droits, qui s'alignent et se raccordent ; un tuyau qui finit sur le flanc d'un autre fait
  un branchement), **vannes** (clic = ouvrir/fermer ; avec l'outil Axe, menées par un engrenage — elles s'ouvrent en N
  tours puis le moteur cale ou repart — ou par un rail), **pompes** (poussent du 1er bout vers le 2e, clic pour inverser
  ou arrêter ; menées par un engrenage, la poussée suit la vitesse et freine le moteur). Chaque bout libre est ouvert :
  il aspire le liquide où il plonge ou le rejette. Le liquide suit la charge hydraulique : vases communicants, **siphon**
  (limité par la hauteur que l'air peut soutenir, ce que la pompe dépasse), **écluse** (deux bassins et une vanne).
- **Vue Pression** (touche P) : pression de l'air (plus mince en altitude, courants), poids des colonnes de liquide,
  poids au-dessus des solides, frontières gaz / liquide / solide ; les tuyaux sont colorés par leur pression.
- **Argile / terre cuite** : l'argile cuit vers 550 °C. Un feu de bois sous un creuset de terre cuite (braises couvertes :
  plus chaudes et plus durables) chauffe le fer et la cendre jusqu'à 400 °C : acier.
- **Pelle** : un clic remplit la bulle (si elle est vide), elle reste pleine en se déplaçant, chaque clic verse une part ;
  clic droit verse tout. Elle garde la chaleur de ce qu'elle porte.
- **Loupe** : le pointeur est une loupe, une vue ronde agrandie en bas à droite montre la matière sous la croix.
- **Sélection** : entoure un élément ; glisser le déplace (les blocs se clipsent), les poignées d'une tige, d'un câble, d'un
  tuyau ou d'un rail les retaillent ; Suppr efface ; Échap désélectionne.
- **Briques** : se clipsent sur les plots (plus de plots en commun = tient mieux) ; une fois clipsées elles ne glissent plus
  de côté, seulement un peu en hauteur et en basculement. Le poteau est la brique haute.
- Le lecteur (jouer, vitesse, voiture) flotte en bas et s'efface quand la souris ne bouge plus ; les messages s'affichent en haut.

## Le Béluga (pour rire)

De temps en temps (rarement, un seul à la fois) un avion Béluga stylisé traverse le ciel. Il suit simplement le champ de
vent : par temps calme il vole droit et traverse l'écran ; ce sont les tourbillons (vortex) de l'air qui le font tourner
et faire des loopings et des culbutes. S'il touche quoi que ce soit (sol, eau, bloc, arbre…) il explose. Touche `B` : en appeler un.

## Vent, saisons et palette

- Onglet Monde, section Vent : trois curseurs. *Force max* plafonne la vitesse du vent, *Virages* règle la fréquence des
  changements de direction (jamais … x3), *Calme-venteux* passe d'un temps calme (petite brise) à un temps venteux.
- Cliquer sur la saison dans la pastille en haut à gauche de la simulation passe à la saison suivante.
- La palette de gauche se cache comme celle de droite : elle sort quand la souris va au bord gauche. La petite poignée en
  haut la garde dehors (et l'image se place à côté).
- Les feuillages laissent passer l'eau : elle goutte à travers les arbres.

## Vue hygrométrie (touche Y, ou Vues > Hygrométrie)

Humidité relative de l'air : la vapeur d'eau et les nuages, rapportés à ce que l'air peut tenir à sa température (l'air
froid en tient peu), plus un fond d'humidité autour des étendues d'eau. Du sec (ocre) à l'humide (bleu). Les liquides sont
toujours à 100 %, la terre montre son humidité, les solides secs sont en gris.

## Voitures, matériaux, palette épinglée

- Une barre flottante en haut de l'image lance une voiture par modèle (un bouton par modèle). Une fois lancée, ses
  réglages sont dans le panneau sous les vues. L'onglet Voitures règle le modèle lui-même (puissance, vitesse, couleur...) :
  les prochaines voitures partent avec ces réglages. Entrée et Sortie y restent.
- « Matériaux illimités » (Édition / Simulation) est dans les palettes Matières et Blocs, modifiable à tout moment.
- La punaise à droite de Annuler épingle la palette (la poignée du bord gauche fait la même chose).
- Le monde remplit toute la fenêtre : un peu de ciel est recadré en haut si la fenêtre n'est pas en 16:9.

## Air compressible (gaz parfait)

Le vent (`src/world/flow.rs`) est un air faiblement compressible : chaque case grossière a une densité `rho` et une température relative `theta` (T/T_ambiante, en kelvin). La pression est `P = rho · theta` (PV = nRT), le vent est poussé par `-∇P`, et le transport de masse (flux amont) change la densité. L'air plus chaud que son voisinage (1 − 1/theta) monte ; une explosion comprime l'air et lance une onde de pression ; une pièce fermée chauffée garde sa surpression un moment. La case « Air compressible » (onglet Monde, section Vent) rebascule sur l'ancien solveur incompressible.

- Le vent transporte aussi la chaleur de l'air libre (advection bilinéaire) et refroidit/réchauffe les solides et liquides au contact (convection, proportionnelle à la vitesse).
- L'humidité de l'air est un champ transporté par le vent : alimenté par l'eau, la boue, les nuages et la vapeur, il sèche lentement. La vue Hygrométrie l'utilise.
- La vue Pression lit la pression de l'air compressible (plus l'hydrostatique des liquides).
- Tout tourne sur CPU ; le GPU ne sert qu'à l'affichage.

## La pression pousse (liquides, matière meuble, blocs)

`src/world/pressure.rs` calcule l'excès de pression (en atm) de chaque case du vent. L'air donne sa valeur au solveur de vent, et un liquide la transmet à toute sa masse connectée (Pascal : relaxation depuis l'air au-dessus), sans saut à l'interface. Le poids propre du liquide n'y est pas compté, la gravité et la poussée d'Archimède l'équilibrent déjà.

- **Liquides et matière meuble** (sable, cendres, neige, poudre…) en bordure d'un gradient glissent vers la basse pression, d'autant plus vite qu'ils sont légers. Un seuil ignore la brise ordinaire, et un seuil plus haut protège le sable que chauffe un feu.
- **Blocs** : le gradient pousse chaque bloc, sur le même principe (une onde de choc pousse, une dépression attire).
- Seule la partie qui varie vite pousse (onde de choc, houle de pression) : la surpression stable d'un panache chaud est retirée par une moyenne lente.
- La vue Pression montre ce champ : hydrostatique des liquides et solides + excès transmis par l'air, lissé latéralement dans les liquides et la roche.

Une explosion sous l'eau lève maintenant un panache : plus elle est peu profonde, plus l'eau de la surface au-dessus est projetée haut (rien si elle est très profonde, la surface se soulève seulement). `--demo out.png --biome mer --boom [x]` déclenche une explosion sous la surface juste avant la capture.

## Garder le jeu fluide : niveaux de simplification

`cargo test --release cost_of_a_tick -- --ignored --nocapture` mesure le coût d'un pas de simulation (campagne : environ 3,4 ms, mer : environ 4,5 ms). La chaleur est le poste le plus lourd : la passe thermique coûte environ 6 ms tous les 3 pas (réactions et relaxation de l'air 2,7 ms, conduction 1,8 ms, transport de la chaleur par le vent jusqu'à 4 ms une passe sur deux), le vent 0,35 ms, la pression moins de 0,1 ms, le reste est l'automate de matière.

Quand les images restent lentes, le jeu simplifie d'abord la physique (`World::lod`), puis seulement après réduit la vitesse. Chaque niveau remplace un calcul par une approximation et une notification l'annonce :

1. la chaleur de l'air est transportée par blocs de 2×2 cellules ;
2. la passe thermique tourne une fois sur deux, toutes ses vitesses (conduction, relaxation, probabilités de réaction) sont doublées pour garder le même rythme ;
3. l'air calme voisin d'un air calme ne conduit plus la chaleur (il se contente de relaxer vers l'ambiant) ; une différence d'au moins 4 °C (un panache chaud) conduit toujours.

Après environ 15 s fluides, un niveau est rétabli (avec un délai de 5 minutes si la simplification venait d'être imposée). Des tests vérifient que le sable fond encore en verre et qu'une pierre chaude refroidit au même rythme à chaque niveau.

## Vapeur et eau sous pression

`src/world/pockets.rs` regroupe le gaz en poches. Une poche qui touche le ciel, les côtés ou le fond, ou qui est très grande, est l'air libre (1 atm). Une poche fermée suit la loi des gaz parfaits (P V = n R T) : sa pression monte avec sa température absolue et avec la vapeur qu'elle contient (une cellule de vapeur compte 8 fois, car l'eau qui bout prend bien plus de place).

- **Chaudière** : une boîte fermée où l'on fait bouillir de l'eau monte en pression. Sa paroi la plus faible cède quand la surpression dépasse ce qu'elle tient (glace 0,6 atm, verre 0,8, terre 2, bois 2,5, pierre 5, fer 8, acier 14) : un craquement pour une petite poche, une explosion pour une grande. Un message dit à quelle pression.
- **Cocotte-minute** : l'eau bout plus chaud sous pression (Clausius-Clapeyron : 100 °C à 1 atm, 120 °C à 2 atm), et aussi au fond d'un lac profond.
- **Eau sous pression** : la pression d'un liquide est celle du gaz sur sa surface plus son propre poids (60 cellules d'eau = 1 atm), partagée dans tout le liquide relié. Un trou dans un mur fait gicler un jet qui garde sa vitesse en tombant, plus loin pour un trou profond (Torricelli). Les vases communicants équilibrent la charge (hauteur + pression), donc une chaudière pousse son eau dans un tuyau qui monte (geyser).
- **Moteur à vapeur** : dans la télécommande d'un moteur, « À vapeur ». Posé contre une chaudière fermée, il tourne avec sa pression (plein couple à 3 atm), use sa vapeur et la rejette par sa cheminée. Sans feu sous la chaudière, il ralentit puis s'arrête.
- **Pas de temps fixe** : la simulation fait 60 pas par seconde (fois la vitesse), quel que soit l'écran (60 ou 144 Hz).
