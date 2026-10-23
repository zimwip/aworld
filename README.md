# AWorld

Bac à sable 2D en Rust (macroquad pour le rendu GPU avec shaders, rapier2d pour les solides) :
on construit un monde, on lance la physique, puis des voitures le traversent (entrée → sortie, façon Lemmings).
Tout est pensé pour apprendre en jouant : température, eau, roches, métaux, plantes, saisons.

```
cargo run --release
```

## Jouer
- **Panneau de gauche** (onglets) : Matières, Blocs, Voitures, Outils, Monde. Survole une case pour lire l'explication.
- **Palette de droite** (Vues) : elle sort quand la souris va au bord droit et se range quand elle s'en va ; la punaise
  (ou un clic sur l'onglet) la garde ouverte, comme la palette de gauche. Normale, Température (°C de chaque cellule),
  Efforts (charge des pièces, vert → rouge), Vent.
- **Barre du haut** : Jouer/Pause, Retour, Annuler (plusieurs fois, même en simulation), Voiture, vitesse, Tourner,
  emplacements 1-2-3, Sauver, Charger, Monde, Vide, Science (toutes les transformations), Plein écran.
- **Construction** : le monde est figé tant qu'on n'a pas lancé. Pause redonne la main pour un 2e tour d'édition.

## Le monde
- **Décors** (Monde → Décor) : campagne, montagne (cimes enneigées), mer. Chacun a son arrière-plan.
- **Jour et nuit**, soleil (la chaleur forme un gradient autour de lui), lune, étoiles (elles scintillent à cause des
  turbulences de l'air, surtout près de l'horizon et par grand vent ; les planètes, elles, brillent sans scintiller) ; **saisons** : les feuilles
  rougissent, tombent, pourrissent en terre ; les arbres repoussent au printemps.
- **Température** : l'air refroidit avec l'altitude ; l'eau bout à 100 °C et gèle à 0 °C, avec un palier (chaleur latente) ; la roche fond vers 540 °C ;
  la lave refroidit très lentement ; le sable chauffé devient verre (et casse sous un choc thermique).
- **Cycle de l'eau** : évaporation → vapeur → nuage (que le vent pousse) → pluie, neige ou grêle selon le froid.
  Un nuage réchauffé par le soleil garde son eau. Les gaz qui sortent d'un côté reviennent de l'autre.
- **Chimie** : bois + feu = cendre (carbone) ; fer + cendre + chaleur = acier ; l'eau rouille le fer ; rouille + feu = fer ;
  eau + lave = pierre ; roche + gel = sable ; verre brisé = éclats puis sable.
- **Poudre** : des feuilles mortes qui pourrissent contre de la cendre de bois donnent du **salpêtre** (la nitrière ;
  à l'abri de la pluie, l'eau le dissout). Salpêtre + charbon de bois, à froid et au sec : de la poudre noire.
- **Clins d'œil** : le Béluga (B) ; une **baleine** souffle dans les grandes étendues d'eau profonde (W pour l'appeler) ;
  un grand feu fait venir le **camion de pompiers** (pin-pon !) qui l'arrose avec de la vraie eau ; la nuit, la poudre
  qui explose lance un **feu d'artifice** (couleurs des métaux : cuivre bleu, strontium rouge, sodium jaune, baryum vert) ;
  de temps en temps, un **petit garçon** (sweat jaune au dinosaure, jogging bleu) vient se promener (K pour l'appeler) :
  il saute les murets, fait la roue et des saltos, et commente ce qu'il voit dans une bulle. Rien ne le brûle ni ne le
  fait exploser, mais un coup, une explosion, du sable sur la tête ou un clic sur lui : « Aïe ! », « Hé ho, ça va pas
  non ?! » (`--demo out.png --kid x[,walk|wave|jump|flip|wheel|ouch]`) ;
  les nuits de **pleine lune**, une meute de **loups** monte sur la plus haute colline et hurle (L pour savoir dans combien
  de nuits) ; le louveteau chante faux et la lune leur fait un clin d'œil.
- **Phases de la lune** : un cycle de 8 jours (une année de jeu) : nouvelle lune, premier croissant, premier quartier,
  lune gibbeuse, pleine lune, puis dernier quartier et dernier croissant. Éclairée à droite quand elle grandit, à gauche
  quand elle décroît. Comme en vrai, elle se lève un peu plus tard chaque jour : la pleine lune se lève au coucher du
  soleil, le premier quartier se voit dès l'après-midi, le dernier quartier ne se lève qu'à minuit et la nouvelle lune
  traverse le ciel avec le soleil (invisible). Le reflet sur la mer et la lumière qu'elle donne suivent sa phase.
  Le nom de la lune s'affiche la nuit dans la barre du monde (`--demo out.png --day 3 --hour 0 --wolves 4`).
- **Plantes** : terre humide + graine + eau + lumière (jamais la nuit) = arbre ; les arbres font des graines : une forêt pousse.
  La terre nue s'érode sous l'eau qui coule (l'herbe la protège). Une graine sans eau pourrit en terre.
- **Pas de vrai sol** : seule la roche posée sur le bas du monde retient le reste ; tout ce qui n'y est pas relié tombe et
  disparaît par le fond. Sable, eau et gaz sortent aussi par les côtés (sauf les gaz, recyclés de l'autre côté) ; le ciel
  bloque les gaz. On peut donc creuser le fond pour vider un lac.
- **Solides** : un solide est un groupe de cellules de même matière qui se touchent (un arbre compte pour un). Il a une
  masse, un centre de gravité et un moment d'inertie (en 2D, la matrice d'inertie se réduit à I = Σ m·r²). Il est
  **ancré** s'il touche le fond du monde, une voûte, ou s'il s'agit d'un arbre enraciné ; même ancré, chaque section doit
  porter le moment de ce qui la dépasse (σ du matériau) : un porte-à-faux trop long casse là où il est le plus faible.
  Un solide **libre** repose sur ce qu'il touche : si son centre de gravité sort de sa base, il **bascule** autour de
  l'arête d'appui, la quitte quand elle ne peut plus le retenir, tombe en tournant et **se brise** à l'impact selon
  l'énergie de rupture du matériau (la glace donne des morceaux et des grêlons, le verre des éclats, la pierre ne casse
  qu'après une grande chute, le bois et le métal jamais). La vue **Efforts** colore chaque section selon sa charge.
  La **terre** n'est pas un solide : c'est un granulat cohésif, elle s'éboule dans le vide et tient un talus raide
  (`--demo out.png --topple` : un long trait de glace dessiné de travers bascule, tombe et se brise).

## Construire
- **Blocs façon Lego** : brique, longue, planche, poutre fer, poutre acier, cube, verre, roue. Aimantés sur la grille de plots
  et sur leurs voisins (Maj = libre), pas de superposition, molette ou Q/E pour tourner. Les blocs qui se touchent sont soudés ;
  bout à bout, les tiges de même matière forment une seule pièce plus rigide.
- **Tiges** (bois, fer, acier) : un clic fixe un bout (sol, bloc ou nœud), tire, un second clic fixe l'autre. Longueur max 110.
  Elles pivotent aux nœuds : on dessine des fermes de pont. Le bois est léger et fragile, le fer lourd, l'acier lourd et solide.
  **Le poids des voitures pèse vraiment sur les tiges** : une longue tige chargée en son milieu plie (comme une planche
  au-dessus d'un ruisseau). Une citadine passe sur une seule tige, même en bois ; un camion demande une tige en acier ou une
  ferme (des triangles) en fer dessous pour raccourcir les portées. Une tige chargée rougit au-delà de la moitié de sa résistance, le tablier s'affaisse (exagéré pour qu'on le
  voie), et près de casser elle tremble et clignote en rouge. La vue Efforts écrit en plus le % de résistance utilisé
  (`--demo out.png --bridge` : un camion sur un pont en acier).
- **Câble** (acier, fer, corde) : deux points — bloc, tige (bout ou milieu), sol ou n'importe quel bord du monde, ciel compris (les tiges, ressorts, élastiques et câbles s'y accrochent). Élastique : il prend la forme
  d'une parabole sous son poids. Le curseur d'élasticité le rend plus ou moins raide ; l'acier tient le plus, la corde s'étire.
  Une tige posée sans attache tombe.
- **Matériaux** : un bloc consomme des cellules de la matière dont il est fait (brique = terre, planche = bois, poutre = fer...).
  En édition on peut choisir « illimité » ; en simulation c'est toujours limité.
- **Marteau** : casse le verre en 3 étapes (bloc → moitiés → éclats → sable) et la glace. Un choc, une chute ou une voiture aussi.
- Le **bois flotte** (une voiture peut rouler dessus, mais ça bouge), le fer et la pierre coulent.

## Voitures
Citadine, Course, 4x4, Camion : suspensions, couple moteur, adhérence. Elles sautent, font demi-tour devant un obstacle, passent
à travers les arbres, brisent le verre, explosent au choc trop fort (contre un mur ou une autre voiture) en projetant leurs morceaux (roues, carrosserie, vitres)
qui retombent, rebondissent, roulent et fument tant qu'ils sont chauds, sont perdues dans le
vide ou en sortant du monde. Leur poids pèse sur les ponts. **Réglages** (onglet Voitures) : puissance, vitesse, suspension,
amortisseurs, adhérence, taille des roues, solidité, poids, couleur — sur la voiture choisie ou pour les suivantes.

## Raccourcis
Espace jouer/pause · B Béluga · W baleine · L loups · K petit garçon · R retour · Z annuler · C voiture · G monde suivant · N vide · H température · X efforts · V vent ·
Q/E/T tourner · Tab onglet · [ ] pinceau · ↑/↓ vitesse · 1-0 matières · F5/F9 sauver/charger · F11 plein écran.

## Développement
`cargo test` (≈70 tests : simulation, voitures, blocs, sauvegarde). `cargo run --release -- --demo out.png [--tab cars]
[--biome mer|montagne] [--hour 18.5] [--season hiver] [--view thermique|efforts|vent] [--palette] [--wreck]` enregistre une capture
(`--wreck` fait exploser une voiture juste avant).


## Son

Tout le son est synthétisé au démarrage (aucun fichier audio) : moteur (4 régimes mélangés selon la vitesse),
vent fort, eau qui coule, feu/lave qui crépitent, explosions, chocs et objets qui tombent, verre qui casse,
tiges/câbles/ressorts qui cèdent, sable projeté. **M** coupe/remet le son ; `--mute` démarre sans son.
Rien à installer pour le son, ni pour compiler ni pour jouer : Windows passe par WASAPI, macOS par CoreAudio, le web
par WebAudio ; sous Linux le jeu charge ALSA (PipeWire/PulseAudio compris) au démarrage. Sans carte son ou sans ALSA,
le jeu tourne simplement en silence. `--no-default-features` compile une version toujours muette.

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

Espace jouer/pause · B Béluga · W baleine · L loups · K petit garçon · R retour · C voiture · Z annuler · H/X/V vues · M son · F ou F11 plein écran · Tab onglet. Poteau (brique haute) dans les blocs : plus besoin de tourner les blocs.
**Ctrl + molette** zoome sur le pointeur (jusqu'à ×8), **Ctrl + glisser** déplace la vue.
**Échap** recule d'un cran : page Science, tracé en cours, vue zoomée (retour à la vue standard), plein écran, puis une fenêtre « Quitter AWorld ? » qui met
la simulation en pause ; Échap y reprend le jeu, **Entrée** quitte.

## Mécanique (onglet Méca)

Pièces : **moteur** (petit engrenage vu de face), **engrenages** petit/moyen/grand, **axe** (relie deux engrenages
éloignés, ou un engrenage à une roue), **coulisseau** (un rail et son chariot), **bielle** (d'un point d'accroche
d'engrenage au chariot), **vis sans fin** (un engrenage tourne la vis d'un rail, le chariot avance) et **pivot** (une roue
sur un axe : poulie, tambour de treuil, roue motrice ; ou le bout d'une tige).

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
- Télécommande du moteur (sélectionne un moteur : menu à part sous les Vues) : en marche/arrêt, sens, vitesse à vide, couple,
  seuil de retournement, et les mesures en direct (tr/min, charge). Vue Efforts : couple, charge des dents, force
  des bielles, pression sur les chariots.
- Câbles : un bout peut rester libre (le second clic dans le vide) ; l'acier est bien plus raide.
- **Vis sans fin** (outil Vis) : un engrenage puis un rail, ou l'inverse. Le rail devient une vis filetée et son chariot
  un écrou : un tour d'engrenage l'avance de 8 cellules, le filet défile avec lui. C'est l'actionneur longitudinal : lent
  mais très fort (il démultiplie le moteur), et irréversible : une charge qui pousse sur l'écrou ne fait pas tourner la
  vis, même moteur arrêté (cric, vérin, ascenseur). L'écrou bute en bout de vis (le moteur cale ou repart) et accepte
  des bielles comme un chariot ordinaire. La gomme sur l'arbre entre l'engrenage et la vis en refait un rail simple.
- **Bras articulés** : l'outil Pivot sur le bout libre d'une tige y pose un axe, en l'air (re-clic pour l'enlever).
  L'outil Axe, d'un engrenage à une tige, fait pivoter la tige autour de son bout fixé : elle suit l'angle de l'engrenage,
  avec la force que le moteur donne à cet arbre (une démultiplication la rend plus forte), et pèse en retour sur le
  moteur. Une tige accrochée au bout d'une autre pivote par rapport à elle : un engrenage par articulation, et on
  monte une grue, une pelleteuse ou un bras robot. Les bielles et les autres tiges complètent la cinématique.

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
- **Tunnel** : creuse comme la gomme, mais pose une voûte maçonnée (briques et joints) au-dessus et sur les côtés du trou.
  La voûte est ancrée dans le terrain : le sable, la terre ou l'eau du plafond ne s'effondrent plus, et un tunnel qui
  traverse une colline de part en part ne fait plus tomber son sommet. La roche tient seule et reste nue ; le sol du tunnel
  n'est pas touché (`--demo out.png --tunnel` : la même dune creusée à la gomme et au tunnel).
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
- Cliquer sur la saison dans la pastille en haut à gauche de la simulation passe à la saison suivante. Un clic ailleurs
  sur la pastille ouvre le **bilan d'énergie** (un second clic le referme) : soleil entrant, sortie vers l'espace, chaleur
  des feux, courbe de la chaleur du monde sur 24 h, et trois curseurs (*Soleil*, *Sortie vers l'espace*, *Nuages et fumée
  gardent*) avec un bouton *Revenir au neutre*. La bulle reprend aussi *Temps*, *Saison* et *Vent* : l'onglet Monde
  donne le point de départ, la bulle permet de jouer avec en pleine simulation.
- **Lance-flammes** (barre d'outils, icône flamme) : garder le clic enfoncé envoie un jet à 1500 °C de la taille du
  pinceau. Le bois s'embrase, la roche rougit puis fond en lave, le fer fond, l'eau bout ; ce qui est lourd chauffe plus
  lentement. Sa chaleur compte dans le bilan d'énergie avec celle des feux.
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
- Les deux menus latéraux marchent pareil : ils sortent au survol, se rangent quand la souris s'en va, et une punaise
  les garde ouverts. À gauche, la punaise est à droite de Annuler (la poignée du bord gauche fait la même chose) ; à
  droite, elle est dans l'en-tête des Vues (un clic sur l'onglet fait la même chose).
- La télécommande d'un moteur (ou les réglages d'une voiture) est un menu à part, sous les Vues : son onglet apparaît
  quand un moteur ou une voiture est choisi, le menu sort un instant pour se montrer, puis marche comme les autres
  (survol, rangement automatique, punaise dans son en-tête ou clic sur l'onglet).
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

## Liquides sous pression : vases communicants, siphons

Case par case, un liquide ne fait que tomber et s'étaler, comme des grains qui ne s'entassent pas : il ne peut pas remonter. Ce qui en fait un liquide, c'est qu'il transmet sa pression. `src/world/hydro.rs` applique une seule règle à tous les liquides (eau, huile, lave, métal fondu), paramétrée par leur densité et leur fluidité :

- Tous les 2 pas, les masses connectées de chaque liquide sont retrouvées. Dans une masse, le liquide passe des endroits où sa charge (hauteur + pression du gaz qui appuie dessus) est la plus haute vers ceux où elle est la plus basse, quelques cases à la fois, d'autant plus vite que l'écart est grand (Torricelli : en racine carrée). Une surface ne donne que si elle est calme (fermée d'une rive à l'autre). L'eau remonte dans un tube en U, deux vases reliés par le fond s'égalisent, une chaudière sous pression pousse son eau dans un tuyau.
- **Embouchures** : le liquide au bas d'un tuyau plein (ou d'un trou étroit, 8 cases au plus) est tenu par la pression derrière lui, comme une paille bouchée du doigt : la colonne ne se casse pas. Si l'embouchure est sous la surface, le liquide s'y écoule (en jet, plusieurs cases d'un coup quand la charge est grande). Un tuyau qui passe par-dessus un rebord fait donc un **siphon** : amorcé (plein), il vide le réservoir jusqu'à son embouchure, puis l'air entre et il s'arrête. Vide, il ne fait rien.
- **Limite de l'air** : l'air ne peut pousser l'eau qu'à environ 10 m (60 cases, `ATM_PER_CELL`) au-dessus de la surface : un siphon dont le sommet est plus haut se désamorce. Plus le liquide est dense, plus cette hauteur est faible (environ 20 cases pour la lave).
- Un filet qui tombe dans l'air (de l'air à côté) ne transmet pas la pression : une cascade ne pompe pas le lac dans le bassin sous elle.
- **Fluidité** : eau 1, huile 0,6, métal fondu 0,3, lave 0,12. La lave s’égalise environ huit fois plus lentement que l’eau, et fige si le tuyau est froid.
- Coût : seules les masses qui touchent une zone éveillée sont parcourues (environ 0,05 ms par pas en campagne, 0,35 ms sur la mer).

`--demo out.png --siphon --advance 900` montre deux vases d'eau, un siphon et un tube en U d'huile après 900 pas.

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

## Zones endormies

`src/world/sleep.rs` découpe le monde en zones de 32×32 cellules. Une zone où rien n'a bougé au dernier pas, et où aucune cellule ne peut changer d'elle-même (roche, sable posé, terre sèche, eau au repos sous de l'eau…), dort : la boucle des cellules la saute. Tout changement de cellule (`set`, `swap`, une pièce qui tombe, un bloc rigide qui bouge) réveille sa zone, et la voisine si la cellule est au bord. La chaleur, le vent et la pression continuent partout. Sur la campagne de `cost_of_a_tick`, une trentaine de zones sur 510 restent éveillées ; la boucle des cellules passe d'environ 0,7 ms à 0,1 ms par pas (mer : 1,8 → 0,8 ms).

## Bilan d'énergie : le soleil entre, l'espace reprend

Le monde est fermé sur les côtés et au fond, ouvert seulement par le haut (`thermal.rs`, `Climate`).

- Le soleil arrive en rayons parallèles depuis sa position : bas et de gauche le matin, d'en haut à midi, de droite le
  soir. Le versant qui lui fait face chauffe, l'autre reste à l'ombre. Les nuages en absorbent une partie, la fumée tout,
  le verre le laisse passer.
- Jour et nuit, la première surface de chaque colonne rayonne vers l'espace l'excès de chaleur qu'elle a sur l'air ; sous
  les nuages et la fumée, moins (effet de serre) : nuits claires froides, nuits nuageuses douces.
- L'air libre suit toujours le cycle habituel du jour et des saisons, mais ce qu'il prend aux cellules (ou leur donne) va
  au reste du ciel, une réserve (`sky_dev`, en °C au-dessus du cycle habituel) qui rend son surplus à l'espace en
  quelques heures. Un grand feu réchauffe donc tout le ciel d'un ou deux degrés, puis tout revient.
- L'air enfermé dans une boîte sans fissure ne suit plus le ciel : il se mélange dans sa boîte et ne perd sa chaleur
  qu'à travers les parois. Une boîte de verre au soleil devient une serre.
- La lave garde environ 16 fois plus de chaleur que la roche (au lieu de 70 fois avant) : elle coule un moment, fond
  encore la roche et le fer qu'elle recouvre, mais une nappe de lave ne contient plus dix fois la chaleur du reste du
  monde.
- Sans rien faire, le cycle est neutre : sur trois jours de jeu, la réserve du ciel reste à moins de 1 °C du cycle
  habituel. Le test ignoré `energy_probe` imprime le bilan toutes les trois heures (`BONFIRE=1` pour un grand feu,
  `FIRE=1` pour une nappe de lave).

## Chaleur latente : les paliers

L'eau gèle et la glace fond à 0 °C. Pour changer d'état, une cellule doit d'abord absorber (ou céder) sa chaleur latente (`World::latent`, `thermal::latent_heat`) : 80 °C de chauffage pour fondre, comme l'eau réelle, et un tiers de la valeur réelle pour bouillir, pour qu'une casserole s'évapore dans un temps raisonnable. Pendant ce temps, sa température ne bouge pas. L'eau qui bout reste à 100 °C (plus sous pression) quelle que soit la chaleur du feu, et la glace qui fond reste à 0 °C : c'est le palier qu'on voit au collège. La chaleur absorbée sans aller jusqu'au bout est rendue si la cellule refroidit. Le soleil sur la glace ou l'eau passe d'abord par ce même compte.

## Le feu a besoin d'air : charbon de bois et acier

- **Le feu respire** : une flamme sans air autour d'elle (seulement de la fumée, de la vapeur ou des murs) s'étouffe en quelques pas. À l'air libre la fumée se dilue ; dans une boîte fermée elle reste (`World::sealed`). Quand la poche fermée est à moitié pleine de fumée, son oxygène est brûlé (`World::stale`) : les flammes s'éteignent et les braises qui restent deviennent du charbon de bois. C'est la bougie sous un verre.
- **Charbon de bois** (nouveau matériau, dans la palette) : le bois chauffé au-delà de 300 °C brûle s'il a de l'air, mais sans air il carbonise et devient du charbon de bois. La meule du charbonnier marche donc : un tas de bois couvert de terre, chauffé, donne du charbon. L'air traverse une épaisseur de cendre, de braise ou de charbon (un tas de braises brûle à cœur), pas une couche de terre.
- **Le charbon brûle plus chaud et plus longtemps** que le bois : une braise de charbon dure plusieurs fois plus longtemps et chauffe 150 °C de plus.
- **Chimie du fer** : la rouille est de l'oxyde de fer ; il faut du carbone pour lui reprendre son oxygène, du charbon de bois tout près ou les flammes et braises d'un feu de bois contre elle. La chaleur seule ne suffit plus. Le fer chaud (400 °C) prend son carbone au charbon de bois ou aux braises proches et devient de l'acier. La cendre, elle, n'a presque plus de carbone (ce sont des minéraux) : elle ne fait plus d'acier, mais elle enrichit toujours la terre.
- **Les tas qui craquent** : une poche de gaz bordée de braises, de charbon ou de cendre fuit par les fissures. Elle ne monte pas en pression et ne fait rien éclater, mais l'air frais n'y rentre pas.

## Évaporation et transpiration des arbres

- **L'eau s'évapore sans bouillir** : une surface libre perd de l'eau à toute température au-dessus de 0 °C. La vitesse suit la pression de vapeur saturante (formule de Magnus, `world::vapour_pressure` : elle double à peu près tous les 11 °C), multipliée par la sécheresse de l'air au-dessus (l'humidité portée par le vent, celle de la vue hygrométrie) et par le vent, qui emporte l'air humide. Un bassin à 30 °C perd environ 3 fois plus d'eau qu'à 5 °C, 6 fois plus qu'à l'air humide, 3 fois moins qu'au vent. L'eau qui reste autour se refroidit un peu (la vapeur emporte sa chaleur latente).
- **La vapeur monte** même refroidie : l'air humide est plus léger que l'air sec (18 contre 29 pour la masse d'une molécule). Elle rejoint l'air froid d'en haut et y devient nuage, puis pluie. Sur une journée de mer, une centaine de cellules d'eau partent en vapeur et une centaine de cellules de nuage se forment.
- **Les arbres transpirent** : le jour, à la belle saison, le pied du tronc boit l'eau du sol (ou d'une flaque) par ses racines, et une feuille au bord de la couronne la rejette en vapeur. Le sol sèche autour des arbres et l'air au-dessus d'une forêt devient humide.
- **Le sol sèche avec l'air** : la terre en surface sèche plus vite à l'air sec et au vent ; enterrée, elle garde son eau bien plus longtemps.

## Grains qui tombent, talus et poussée d'Archimède

- **Les grains accélèrent** en chute libre : chaque grain garde sa vitesse de chute (`World::vel`, en huitièmes de cellule par pas). Elle augmente à chaque pas (la gravité) jusqu'à une vitesse limite de 5 cellules par pas (le frottement de l'air), et retombe à zéro quand le grain se pose.
- **Chaque matériau a son angle de talus** : le sable sec coule en tas plats (environ 33°, comme le vrai), le sable mouillé et la neige tiennent des pentes raides (environ 58° pour la neige), la grêle et la boue s'étalent. Certains grains d'un tas ne glissent que s'il y a deux cellules de chute, d'autres roulent de deux cellules : lesquels dépend de leur place (`World::grain_pick`), donc le tas se pose à un angle stable.
- **Poussée d'Archimède** : un grain plus léger que le liquide qui l'entoure remonte (charbon de bois, cendre, feuilles mortes, grêle flottent sur l'eau), un plus lourd coule. Un bloc qui tombe fait pareil selon sa densité moyenne : une rondelle de bois flotte comme la glace, un rocher coule au fond du lac en poussant l'eau (avant, il restait posé sur l'eau).

## Dessin : ne recolorier que ce qui change

- Avant, chaque image recalculait la couleur des 518 400 cellules (environ 3 ms sur un cœur) et rebalayait tout le monde pour retrouver les arbres (0,5 ms). Maintenant le dessin ne recolorie que les zones de 32×32 qui ont tourné ou ont été touchées depuis la dernière image (`Zones::changed_since`), plus une rangée de zones à tour de rôle pour les changements lents qui ne réveillent pas une zone (teinte de la chaleur, humidité du sol) : tout est rafraîchi au moins toutes les 17 images. Sur la campagne, une soixantaine de zones sur 510 par image, soit environ 10 fois moins de travail. Les arbres et les nuages suivent la même règle, zone par zone.
- Le scintillement de l'eau et des braises, de la lave et des flammes est passé dans le shader (une graine de bruit par cellule et le temps) : une cellule qui ne change pas garde exactement la même couleur dans la texture, ce qui rend le recoloriage partiel exact.

## Capteurs : thermomètre, baromètre, hygromètre

Le nouvel outil **Capteur** (barre d'outils à gauche, icône thermomètre) pose une sonde dans le monde, jusqu'à 4 (A, B, C, D). Chaque sonde relève toutes les 30 étapes (une demi-seconde de jeu) la température, la pression absolue (celle de la poche de gaz, ou dans un liquide le poids de ce qui est au-dessus plus le gaz qui appuie) et l'humidité de l'air. Ses courbes des deux dernières minutes s'affichent dans une carte en bas de l'image, chaque courbe à sa propre échelle pour qu'un petit changement se voie. On y voit l'eau d'une casserole chauffer sans jamais dépasser 100 °C, une chaudière monter en pression, l'air sécher à midi. Cliquer sur une sonde l'enlève ; « Retour » efface les courbes mais garde les sondes. Les sondes ne sont pas enregistrées dans les sauvegardes.

## Lumière et effets (shaders)

- **Reflets** : la surface de l'eau scintille au soleil (paillettes plus nombreuses sous le soleil), et à la lune la nuit.
- **Lumière du feu** : la nuit, le feu, les braises et la lave éclairent tout autour d'eux d'une lumière chaude qui vacille.
- **Air chaud** : au-dessus d'un feu ou de la lave, l'air chaud fait onduler ce qu'on voit à travers (comme au-dessus d'une route en été).
- **Aube et crépuscule** : quand le soleil est bas, le ciel passe à l'orange près de l'horizon, au rose au-dessus et au violet en haut, avec un halo doré du côté où il se lève ou se couche. Le ciel ne s'éteint pas d'un coup : la lueur reste un moment après le coucher. Les nuages sont éclairés par en dessous et sur la face tournée vers le soleil, en or près de lui et en rose plus loin, leur sommet dans une ombre lavande.
- Une petite carte de lumière (une case pour 6×6 cellules) suffit : tout reste fluide.

## Explosions et cratères

`src/world/blast.rs` fait le bilan d'énergie d'une explosion, avec la même règle pour tous les matériaux.

- **Toute la charge part ensemble** : les grains de poudre qui se touchent brûlent dans le même instant, l'explosion a l'énergie de toute la charge (3 MJ/kg de poudre noire, dont 1 % travaille sur le sol ; le reste chauffe le gaz, fait de la lumière et du bruit). Une longue traînée fine part par morceaux, comme une mèche.
- **Arracher une case coûte** `V · (σ + ½ ρ (v² + 2 g h))` : `σ` la résistance du matériau (MPa, la cohésion pour un sol), `ρ` sa densité, `v` la vitesse à laquelle il faut le lancer pour sortir du trou et `h` la hauteur dont il faut le lever. Le sable ne coûte que son poids et son lancer, la terre un peu plus, la roche bien plus, l'acier tient presque. Un nouveau matériau n'a besoin que de sa densité et de sa résistance.
- **La ligne de moindre résistance** : l'énergie part en rayons depuis le centre, et un rayon qui atteint l'air libre à travers peu de sol en reçoit davantage. Ceux qui percent projettent ce qu'ils ont arraché avec l'énergie qui leur reste ; ceux qui s'épuisent sous terre écrasent seulement le sol (une cavité cachée, le « camouflet »).
- **Les éjecta volent** (balistique, gravité), retombent et roulent comme du sable : ils font le bourrelet autour du cratère. La terre qui tombe dans l'eau devient de la boue.
- Le gaz de la charge (environ 280 L par kg) monte la densité de l'air autour : l'onde de pression du vent compressible pousse ensuite les blocs, le sable et l'eau. Le rayon du feu, du souffle et de la secousse grandit avec la racine de la quantité de poudre.

- **Ce qui ne peut pas sortir reste** : la matière écrasée d'un camouflet n'est pas supprimée, seul le gaz de la charge ouvre une cavité. Le gaz qui s'échappe par le haut pousse encore sur les parois voisines, ce qui élargit le trou en cône, et les éjecta partent vers 45° pour retomber au-delà du bourrelet.
- **Poudre et dynamite** sont deux explosifs de la même table (`explosive()`), décrits par leur énergie, leur densité, leur part d'énergie transmise au sol, leur gaz, leur pic de pression et la température de leurs gaz. La poudre noire déflagre (environ 3 000 atm) ; la dynamite détone (plus de 100 000 atm) et creuse mieux pour la même masse.
- **L'onde de choc** (pression de pic qui décroît comme le cube de la distance) fait passer la matière qu'elle traverse par la table des changements d'état : la terre se tasse en argile près d'une détonation, et le charbon de bois devient du **diamant** au-delà de 50 000 atm et 1 500 °C, comme les nanodiamants de détonation. La poudre noire n'en fait jamais. Ce qui est projeté est desserré, seul le changement de cristal (le diamant) y survit.

`--demo out.png --crater n` enterre une grosse charge de poudre sous une prairie et l'allume `n` images avant la capture ; avec `--dynamite`, c'est un bâton de dynamite enrobé de charbon de bois.

## Changements de matière (table commune)

`src/world/matter.rs` regroupe les changements d'une matière en une autre dans une seule table `TRANSITIONS` : de quoi, en quoi, et sous quelles conditions (`TempAbove`, `TempBelow`, `PressureAbove` pour la pression qui dure, `ShockAbove` pour le pic d'une onde de choc, `Under` pour ce qui est posé dessus). `World::change_state` fait le changement en gardant la chaleur de la cellule.

- **Pression lithostatique** : une atmosphère d'air plus le poids de la colonne de matière au-dessus de la cellule (`World::load`).
- **Tassement** : la terre sous l'eau calme devient de la vase ; la vase enfouie sous assez de poids devient de l'argile ; l'argile et le sable sous un grand poids deviennent de la roche (schiste, grès) ; la neige profonde devient de la glace. Dans la réalité, cela demande des kilomètres de terrain et des millions d'années. Le monde n'a que 27 m de profondeur : les seuils sont abaissés et les changements arrivent en quelques heures de jeu.
