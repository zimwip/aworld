# AWorld

Bac à sable 2D en Rust (winit pour la fenêtre, wgpu pour le rendu avec shaders et le calcul des liquides sur la carte, rapier2d pour les solides) :
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
- **Glisser un outil** (pinceau, lance-flammes…) : tant que le bouton reste enfoncé, les menus laissent passer la
  souris (ils ne s'ouvrent pas, ne s'allument pas, ne cliquent pas) et celui que l'outil survole devient transparent,
  pour voir le monde dessous et continuer le tracé (`--demo out.png --palette --mouse 1400,300 --stroke`).
- **Barre du haut** : Jouer/Pause, Retour, Annuler (plusieurs fois, même en simulation), Voiture, vitesse, Tourner,
  emplacements 1-2-3, Sauver, Charger, Monde, Vide, Science (toutes les transformations), Plein écran.
- **Construction** : le monde est figé tant qu'on n'a pas lancé. Pause redonne la main pour un 2e tour d'édition.

## Le monde
- **Décors** (Monde → Décor) : campagne, montagne (cimes enneigées), mer. Chacun a son arrière-plan.
- **Jour et nuit**, soleil (la chaleur forme un gradient autour de lui), lune, étoiles (elles scintillent à cause des
  turbulences de l'air, surtout près de l'horizon et par grand vent ; les planètes, elles, brillent sans scintiller) ; **saisons** : les feuilles
  rougissent, tombent, pourrissent en terre ; les arbres repoussent au printemps.
- **Température** : l'air refroidit avec l'altitude ; l'eau bout à 100 °C et gèle à 0 °C, avec un palier (chaleur latente) ; la roche fond vers 540 °C ;
  la lave refroidit très lentement ; le sable chauffé devient verre (et casse sous un choc thermique, sauf chaud et mou) ;
  vers 850 °C le verre fond en verre liquide, épais, qui se fige en refroidissant et se mélange à la lave qu'il touche.
- **La terre est un mélange** : du sable, de l'humus et de l'eau. Chauffée (contre de la lave), son eau bout d'abord et la
  tient à 100 °C (la vapeur traverse le sol et sort à la surface) ; sèche, son humus brûle vers 250 °C en fumant et elle
  pâlit ; sans humus il ne reste que le sable, qui s'écroule, devient verre puis fond dans la lave. Sèche et sans humus,
  la terre tient moins bien (`--demo out.png --soil --advance 4000`).
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
- **Blocs façon Lego** : brique, longue, planche, poutre fer, poutre acier, cube, verre, roue. Posés librement au pixel près ;
  près d'un autre bloc ils s'aimantent (collés contre lui, bords ou milieux alignés ; une brique emboîtée sur une autre est toujours calée pile sur ses plots) et de
  fins pointillés montrent l'alignement (Maj = sans aimant). Pas de superposition, molette ou Q/E pour tourner. Les blocs qui se touchent sont soudés ;
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
- **Verre** : il casse en 3 étapes (bloc → moitiés → éclats → sable) sous un choc, une chute, une voiture ou une explosion.
- **Dynamite** (outil, elle n'est plus dans la palette des matières) : un clic lâche un bâton à mèche qui tombe à travers l'air
  et l'eau et se pose sur la matière (ou sur un autre bâton). Cliqué dans la roche, la terre ou un mur, il y reste planté et saute
  de l'intérieur, comme dans un trou de mine. La mèche brûle 3 s quand la simulation tourne (en construction, le bâton attend et
  fait partie du plan : Réessayer le remet). Les bâtons qui se touchent sautent en une seule charge, plus forte, et l'onde de choc
  fait partir les bâtons voisins quelques pas plus tard (détonation par influence) : un tas saute presque d'un coup.
  Le bâton est posé par-dessus la matière, sans la remplacer : seule son explosion la transforme. Des bâtons plantés
  dans le même trou s'additionnent. Dans la roche, l'onde de choc broie la pierre en gravats autour de la charge (zone
  broyée, là où le choc dépasse sa résistance à la compression) et la fend en fissures rayonnantes plus loin (là où il
  dépasse sa résistance à la traction) : la roche est pulvérisée de l'intérieur, comme dans une carrière.
  Verrouillée pendant le Défi, comme la construction (`--demo out.png --sticks --frames 120`,
  `--demo out.png --quarry --frames 400` : un bâton seul, quatre dans le même trou et trois en chaîne dans la roche).
- Le **bois flotte** (une voiture peut rouler dessus, mais ça bouge), le fer et la pierre coulent.

## Voitures
Citadine, Course, 4x4, Camion : suspensions, couple moteur, adhérence. Elles sautent, font demi-tour devant un obstacle, passent
à travers les arbres, brisent le verre, explosent au choc trop fort (contre un mur ou une autre voiture) en projetant leurs morceaux (roues, carrosserie, vitres)
qui retombent, rebondissent, roulent et fument tant qu'ils sont chauds, sont perdues dans le
vide ou en sortant du monde. Leur poids pèse sur les ponts. **Réglages** (onglet Voitures) : puissance, vitesse, suspension,
amortisseurs, adhérence, taille des roues, solidité, poids, couleur — sur la voiture choisie ou pour les suivantes.

## Raccourcis
Espace jouer/pause · B Béluga · W baleine · L loups · K petit garçon · R réessayer · Z annuler · C voiture · G monde suivant · N vide · H température · X efforts · V vent ·
Q/E/T tourner · Tab onglet · [ ] pinceau · ↑/↓ vitesse · 1-0 matières · F5/F9 sauver/charger · F11 plein écran · F3 compteur d'images · Ctrl+D métriques de la simulation (débogage).

## Développement
`cargo test` (≈70 tests : simulation, voitures, blocs, sauvegarde). `cargo run --release -- --demo out.png [--tab cars]
[--biome mer|montagne] [--hour 18.5] [--season hiver] [--view thermique|efforts|vent] [--palette] [--wreck]` enregistre une capture
(`--wreck` fait exploser une voiture juste avant).

**Taille du monde** : 960 × 540 cellules par défaut. Les curseurs Largeur / Hauteur du menu du climat (clic sur la
pastille de l'heure) relancent le jeu avec un monde neuf d'une autre taille, de 384 × 216 à 1920 × 1080 (pas de 96
colonnes et de 18 lignes) ; au lancement : `cargo run --release -- --grid 1440x810` (ou `AWORLD_GRID=1440x810`). Le
relief et le ciel s'étirent avec le monde ; les objets (voitures, blocs, arbres, personnages) gardent leur taille en
cellules. Une sauvegarde ne se charge que dans un monde de sa taille. Pas dans le navigateur pour l'instant.

**Les unités du monde** (`src/world/units.rs`) : la grille de référence fait 960 colonnes de 5 cm (48 m de large), un
tick dure 1/60 s, g vaut 9,81 m/s². Un monde d'une autre largeur est le même paysage en cellules plus fines ou plus
grosses : 2,5 cm à 1920 colonnes, 12,5 cm à 384 (la taille d'une cellule est fixée au lancement, avec celle de la
grille). Tout ce qui bouge par une loi physique en tire son échelle : les liquides (pression en pascals, vitesses en
m/s), les grains et les gouttes qui tombent (accélération g, vitesses limites en m/s : 9 m/s pour le sable et la
pluie, 0,9 m/s pour la neige), les morceaux de structure qui tombent, les dalles qui basculent, les poches de gaz
(une colonne d'eau de 10,3 m, 206 cellules, fait une atmosphère), les souffles d'explosion, la dynamite, les débris,
les voitures, les blocs (Rapier garde son unité de 16 cellules, ses forces et raideurs suivent g), le petit garçon,
les feux d'artifice et la baleine (leur allure suit √g). La conduction de la chaleur suit la finesse de la grille.
Les shaders des liquides reçoivent la même taille de cellule. Les grilles grossières gardent leur taille en mètres :
la maille du vent fait environ 40 cm (8 cellules à 960, 16 à 1920, 3 à 384, le diviseur de la largeur le plus
proche) et ses vitesses sont les mêmes en m/s, le carré des charges d'orage environ 30 cm. Les taux qui font avancer
quelque chose d'une cellule à la suivante (le feu et les braises qui gagnent le bois, l'herbe qui s'étend, l'eau que
le sol boit ou qui en goutte, la neige qui flue, une bulle qui remonte) sont d'autant plus fréquents que la cellule
est petite, l'humidité qui diffuse dans le sol comme le carré, l'évaporation d'une surface comme la finesse
(`World::chance_per_cell`). La sonde `grids_probe` (dans `bench.rs`, à lancer seule, une fois par taille :
`AWORLD_TEST_GRID=1920x1080 cargo test --release --bin aworld -- --ignored --exact world::bench::grids_probe
--nocapture`) refait les mêmes scènes en mètres : à 384, 960 et 1920 colonnes, un grain lâché de 4 m touche le sol
en 0,90 s, le front d'un mètre d'eau lâché est à 3,1 m après une seconde, le feu parcourt 2 m d'une planche en 16 à
19 s, le vent à pleine force souffle à 1,5 m/s ; l'évaporation d'une mare reste du même ordre (1,0 à 2,1 mm par
minute), l'humidité de l'air sur la maille du vent dépendant de la place de la surface dans la maille. Restent
liés à la cellule : les gaz et les flammes avancent d'une cellule par tick au plus (la fumée et la vapeur montent
plus lentement en m/s sur une grille fine), la pousse des plantes, la durée de vie d'une flamme ou d'une braise, et
la taille des objets (voitures, arbres, personnages) en cellules.

**Une fiche par matière** (`src/world/spec.rs`, `MatSpec`) : densité (kg/m³), résistance (MPa), ténacité (J/m²),
pression que tient un mur d'une cellule autour d'une poche de gaz, ce que la matière devient broyée, restitution et
porosité, dans un seul `match` sans bras par défaut (une matière nouvelle ne compile pas sans sa fiche). La chute et
la flottaison, les explosions, les structures, les poches de gaz et le poids de la matière sur le sol y lisent tous ;
c'étaient des tables à part qui ne disaient pas toujours pareil. La glace broyée donne de la grêle partout (de la
neige sous une explosion ou dans une chaudière qui crève, avant), l'argile broyée de la terre (une explosion laisse
l'argile qu'elle a tassée), et un mur ne tient jamais plus de pression que la résistance de sa matière (un test le
vérifie : la terre et l'herbe tenaient 2 atm pour une cohésion de 0,5 et 0,8).

**Le mouvement décidé par le type** : la fiche dit aussi comment le tour des cellules déplace la matière (`Motion` :
grain, motte de terre, gazon, liquide, gaz, nuage, flamme, fixe) et sa vitesse limite de chute (`fall_ms`). Le tour
d'une cellule fait agir la matière (`act` : ce qu'elle fait d'elle-même, brûler, se dissoudre, être tenue par
l'aimant), la déplace selon sa façon de bouger (`move_cell`), puis, si elle n'a pas bougé, ce qu'elle fait au repos
(`at_rest` : le sol qui boit et pousse, la neige qui flue, les solides). Un grain de plus n'a besoin que de sa fiche.
Le changement n'a rien changé au monde : l'empreinte des scènes de `bench_fingerprint` est restée la même.

**Ce qui vole hors de la grille** (`src/world/ballistic.rs`) : les gouttes des liquides, les mottes des explosions,
le jet des pompiers, les débris de voiture et les bâtons de dynamite volent de la même façon, la gravité du monde et
une traînée en carré de la vitesse réglée par leur vitesse limite (`pull` : 9,1 m/s une goutte, 30 m/s une motte,
15 m/s une tôle, 12 m/s un bâton ; le jet, un filet d'eau, sans traînée), et un trajet cellule par cellule qui
s'arrête à la première qui bloque (`sweep`). Chacun garde ce qu'il fait en arrivant.

**Morceaux qui tombent et dalles** : deux façons de faire tomber un corps détaché, gardées toutes deux. Un morceau
(`Piece`) descend tout droit dans la grille, flotte ou coule dans les liquides en les écartant, s'enfonce dans le
sable, casse les feuillages et peut être immense ; une dalle (`Slab`) est sortie de la grille et tourne, roule et
rebondit dans l'air, et redevient un morceau dans un liquide. Ce qu'ils calculent tous deux l'est une fois : la
vitesse en cellules par tick (un morceau passe à une dalle et revient sans conversion), la masse et le centre de
masse, la densité moyenne, la casse à l'impact (`fracture`) et les chocs pour le son (`thud`) ; la cadence des
chutes vient de l'ordonnanceur.

**Les cadences** (`src/world/cadence.rs`) : chaque passe qui ne tourne pas à chaque tick (structures, chutes,
poches, transformations lentes, pression de l'air, saison, orage, flux d'air) a sa période en secondes du monde et
sa phase dans un seul tableau ; un test vérifie que les passes lourdes ne tombent jamais sur le même tick.

**Les réglages par l'environnement** (`src/config.rs`) : toutes les variables `AWORLD_*` sont lues une fois, au même
endroit. Le jeu : `AWORLD_GRID=LxH` (taille du monde, après `--grid`), `AWORLD_GPU=1|auto` (la pression des liquides
sur la carte), `AWORLD_GPU_MIN` (inconnues qu'il faut pour aller sur la carte), `AWORLD_GPU_FUSE` (niveau du
multigrille fait en un seul lancement), `AWORLD_CELLS_PAR=0` (le tour des cellules sur un seul cœur),
`AWORLD_PROFILE` (le temps de chaque partie d'un pas, au journal ; `0` l'éteint), `AWORLD_DEBUG_MASS` (les liquides
vérifient qu'ils ne gagnent ni ne perdent rien), `AWORLD_DEBUG_CARD` (ce que fait la carte, et ses passes chronométrées).
Les bancs et sondes des tests : `AWORLD_BENCH_STEPS`, `AWORLD_BENCH_SCENE`, `AWORLD_BENCH_DT`, `AWORLD_BENCH_TIER`, `AWORLD_BENCH_SERIES`,
`AWORLD_BENCH_GRID`, `AWORLD_FLAME_DIR`, `AWORLD_SEEDS`, `AWORLD_PROBE_DT`, `AWORLD_PROBE_STEPS`, `AWORLD_FLOOD_DIR`,
`AWORLD_TEST_GRID`, `AWORLD_SNAP`. Quelques sondes de laboratoire gardent leurs propres boutons (`WIND`, `PHASE`, `DAYS`…).


## Son

Tout le son est synthétisé au démarrage (aucun fichier audio) : moteur (4 régimes mélangés selon la vitesse),
vent fort, eau qui coule, feu/lave qui crépitent, explosions, chocs et objets qui tombent, verre qui casse,
tiges/câbles/ressorts qui cèdent, sable projeté. **M** coupe/remet le son ; `--mute` démarre sans son.
Rien à installer pour le son, ni pour compiler ni pour jouer : un seul petit mélangeur (`src/sound/out.rs`) joue par
cpal sur Windows (WASAPI), macOS (CoreAudio) et le web (WebAudio, après le premier clic ou la première touche, comme
le demandent les navigateurs) ; sous Linux le jeu charge ALSA (PipeWire/PulseAudio compris) au démarrage. Sans carte
son ou sans ALSA, le jeu tourne simplement en silence. `--no-default-features` compile une version toujours muette.

## Sol meuble et vent

- Sable, boue, neige, terre et herbe sont meubles pour les voitures : un peu de traînée, de l'adhérence en moins,
  du sol projeté derrière les roues, et une voiture lente qui patine peut s'enliser (elle se dégage dès qu'elle repart).
- Le vent pousse aussi les blocs libres (surface × vent²), et les arbres se penchent et ondulent avec lui
  (effet visuel, plus fort en haut).
- Le vent passe par-dessus l'eau posée (une mare est un obstacle pour lui, pas la pluie qui tombe). Avec la
  physique des liquides, il entraîne la surface (l'eau monte sous le vent) et lève des vagues : des rafales de
  pression qui courent à la vitesse des vagues. Une eau à l'abri (grotte, cuve, sous un pont) reste calme.
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
  n'importe quel hébergeur statique. Il faut `rustup` (le script installe la cible `wasm32-unknown-unknown`, et la
  première fois l'outil `wasm-bindgen` de la même version que le jeu, avec `cargo install`). Le navigateur dessine en
  WebGPU, ou en WebGL 2 s'il n'a pas WebGPU ; les liquides s'y calculent sur le processeur.
- Sous Linux, la fenêtre a besoin de `libxkbcommon-x11` (présent sur tout bureau ; sur un serveur sans écran :
  `apt install libxkbcommon-x11-0`), et la carte graphique d'un pilote Vulkan (ou OpenGL ES).
- Version web : les sauvegardes restent en mémoire tant que la page est ouverte (pas de fichiers).
- `install` copie le jeu dans `~/.local/share/aworld` (Linux) ou `%LOCALAPPDATA%\AWorld` (Windows), où vivent aussi
  les mondes sauvegardés. Sous GNOME, si le raccourci du bureau reste grisé : clic droit, « Autoriser le lancement ».
  Les scripts Windows n'ont pas pu être essayés depuis Linux.

## Raccourcis

Espace jouer/pause · B Béluga · W baleine · L loups · K petit garçon · R réessayer · C voiture · Z annuler · H/X/V vues · M son · F ou F11 plein écran · F3 compteur (FPS : images par seconde, PPS : pas de simulation par seconde, temps d'un pas) · Ctrl+D métriques de la simulation (débogage) · Tab onglet. Sur ordinateur, le monde tourne dans son propre fil d'exécution et l'écran affiche la dernière image du monde à son rythme : un monde lourd fait baisser les PPS, pas les FPS (`--one-thread` revient à une seule boucle, pour comparer ; le web garde une seule boucle). Poteau (brique haute) dans les blocs : plus besoin de tourner les blocs.
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
  (limité par la hauteur que l'air peut soutenir, une dizaine de mètres d'eau quelle que soit la grille, ce que la pompe
  dépasse), **écluse** (deux bassins et une vanne). La pression du gaz sur l'eau compte dans la charge : une chaudière
  ou un réservoir fermé sous pression pousse son eau dans un tuyau plus haut que sa surface.
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
  donne le point de départ, la bulle permet de jouer avec en pleine simulation. En bas, **Matières du monde** : le
  volume de chaque matière présente, de la plus abondante à la moins (une cellule est un cube de 5 cm, 0,13 L, sur la grille de référence ; une
  cellule de liquide compte ce qu'elle contient vraiment, la vapeur tassée compte toute sa vapeur), chacune avec sa
  courbe sur 24 h. Compté toutes les demi-secondes, et à chaque relevé de la chaleur pour les courbes
  (`--demo out.png --climate --advance 1200`).
- **Lance-flammes** (barre d'outils, icône flamme) : garder le clic enfoncé envoie un jet à 1500 °C de la taille du
  pinceau. Le bois s'embrase, la roche rougit puis fond en lave, le fer fond, l'eau bout ; ce qui est lourd chauffe plus
  lentement. Sa chaleur compte dans le bilan d'énergie avec celle des feux. C'est lui qui allume : le feu n'est plus dans la palette des matières.
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

Tous les liquides (eau, huile, lave, métal fondu) sont mus par leur physique (`src/world/liquid.rs`, voir plus bas « Liquides physiques ») : une vitesse sur chaque arête, une pression résolue en pascals dans chaque cellule. Les vases communicants, les tubes en U et les siphons en sortent d'eux-mêmes ; l'air ne pousse l'eau qu'à environ 10 m (206 cellules de 5 cm) au-dessus de la surface. L'ancien mode où chaque case de liquide était une particule, nivelée tous les deux pas par `hydro.rs`, a été retiré : les cellules ne font plus que trier les liquides par densité (l'huile remonte à travers l'eau).

`--demo out.png --siphon --advance 900` montre deux vases d'eau, un siphon et un tube en U d'huile après 900 pas.

## Garder le jeu fluide : niveaux de simplification

`cargo test --release cost_of_a_tick -- --ignored --nocapture` mesure le coût d'un pas de simulation (campagne : environ 3,4 ms, mer : environ 4,5 ms). La chaleur est le poste le plus lourd : la passe thermique coûte environ 6 ms tous les 3 pas (réactions et relaxation de l'air 2,7 ms, conduction 1,8 ms, transport de la chaleur par le vent jusqu'à 4 ms une passe sur deux), le vent 0,35 ms, la pression moins de 0,1 ms, le reste est l'automate de matière.

`cargo test --release --bin aworld bench_scenes -- --ignored --nocapture` fait tourner six scènes chargées (démo, explosions, liquides, orage, incendie, nuage) et donne le temps de chaque phase du pas (`src/world/prof.rs`, aussi dans le jeu avec `AWORLD_PROFILE=1`) ; `AWORLD_BENCH_SERIES=1` imprime les pas un à un.

**Métriques de la simulation (Ctrl+D)** : un panneau au-dessus du compteur dit où passe le temps d'un pas, partie par partie (`src/metrics.rs`) : le monde et ses phases (cellules, chaleur et ses passes, liquides et leurs passes, pression…), la dynamite, les capteurs, les voitures, les débris, les blocs et ponts, les mécanismes, les tuyaux, les empreintes des blocs, les suites (feu, rouille, explosions) et les pompiers. Pour chacune : millisecondes par pas, part du pas et pire pas sur la dernière seconde (rouge au-delà de 16,7 ms, le budget d'un pas à x1). Au-dessus du tableau, le pas moyen, le pire pas, la part du temps passée à simuler, les zones éveillées, le niveau de simplification, le nombre d'objets, et un graphe des 240 derniers pas avec la ligne du budget ; en bas, le temps par image passé autour des pas (animaux, petit garçon…), à dessiner le monde et à dessiner l'interface. Caché, le panneau ne lit aucune horloge. `AWORLD_PROFILE=1` imprime les mêmes chiffres dans le terminal tous les 600 pas ; `--demo out.png --metrics` montre le panneau sur une capture. **Les pas sont lissés** : la passe de chaleur est répartie sur les trois pas de son cycle (conduction avec l'air et les réactions, puis espace et soleil, puis le vent une fois sur deux), et les passes qui reviennent tous les quelques pas sont décalées pour ne jamais tomber ensemble (structures tous les 9 pas, poches tous les 18, orage et pression de l'air tous les 4, à des phases différentes). Avant, un pas sur trois coûtait dix fois les deux autres et le pire pas de la démo atteignait 29 ms ; il est à 13 ms pour la même moyenne.

**Liquides physiques** (seul mode depuis le retrait des liquides en particules) : la pression est résolue jusqu'à convergence par gradients conjugués préconditionnés par un multigrille géométrique (`src/world/mgpcg.rs`, d'après McAdams, Sifakis et Teran 2010) : les grilles grossières portent la correction d'un bout à l'autre du bassin en quelques balayages, les fines la polissent, et le départ de la pression du sous-pas précédent fait qu'un bassin au repos est réglé en une itération. Les anciens balayages rouge-noir n'avaient jamais le temps de converger sur un bassin large, et la pression fausse faisait lentement gonfler ou dégonfler le liquide. Trois à quatre itérations suffisent (tolérance relative 10⁻³, absolue 1 Pa), chacune un cycle en V sur des listes compactes de cellules par couleur. Avec la physique des liquides, les cellules ne jouent plus le tour des liquides, qui était ce qui sortait différemment sur plusieurs cœurs : le tour des cellules passe alors sur tous les cœurs par défaut (`AWORLD_CELLS_PAR=0` pour l'éviter). Sur la scène « liquides » (60 pas, 4 cœurs) : 70 ms par pas avant, 61 après, pour une pression juste ; les passes de transport sont fusionnées (onze à cinq), et une cellule de liquide endormie ne donne plus de liquide (elle était hors des rangées travaillées, et ce qu'elle donnait n'était jamais débité).

**La chaleur de l'air et la pluie en physique** : la relaxation de l'air (chaleur de l'air et réactions) passe sur tous les cœurs, rangée par rangée, et ne visite plus que les cellules qui peuvent réagir à leur température (une plage de repos par matière : la neige sous −2 °C, le bois loin de son point de feu, etc. ; les zones sans cellule active sont sautées) ; de 3,96 à 1,46 ms la passe (démo 6,1 → 4,7 ms par pas). La seule différence : la chaleur qu'une réaction lâche sur ses voisines arrive après leur relaxation du même pas et non avant. Le vent que lisent les gaz et les gouttes est tabulé par blocs de 4×4 cellules à chaque pas du vent (`Flow::wind_at`) au lieu d'être interpolé cellule par cellule (tour des cellules de l'orage 8,0 → 6,7 ms). Avec la physique des liquides, la pluie sur le champ coûtait un prix fixe par sous-pas, pas lié au peu de liquide qui court : le solveur n'empile plus que les niveaux que la taille du système demande, ses passes de construction et de remise à zéro sont fusionnées, les forces et le reclassement entre deux sous-pas ne touchent que les travées de liquide actif (et une cellule autour) ; 2,7 → 1,5 ms par résolution, et l'orage en physique passe de 31 à 17 ms par pas (liquides 21 → 7,5).

**Une projection par pas** : la pression des liquides n'est plus résolue qu'une fois par pas, au dernier sous-pas ; les sous-pas (jusqu'à quatre, pour que le liquide ne traverse qu'une cellule à la fois) ne font que les forces, la viscosité et le transport, et la pression du pas précédent tient le liquide pendant les premiers. Sur la scène « liquides » (4 cœurs) la pression passe de 42 à 16,5 ms par pas (1046 résolutions pour 300 pas avant, 300 après), le barrage, le tube en U et l'éclaboussure gardent leur vitesse, leur période et leur volume. Une résolution allégée (deux itérations) dans les sous-pas intermédiaires ne rapportait rien : c'est le nombre de résolutions qui compte, pas leur profondeur.

**La pression des liquides sur la carte graphique** (`src/world/gpu.rs`, `src/world/mgpcg_gpu.rs`, noyaux `src/world/mgpcg.wgsl`) : le même multigrille et les mêmes gradients conjugués, chaque passe un noyau de calcul wgpu (Vulkan, Metal ou DirectX 12 selon la machine) sur une grille entière de la hiérarchie. Le niveau fin est construit sur le CPU comme avant et envoyé entier ; la carte construit les niveaux grossiers, fait les itérations et rend la pression. Le CPU attend la carte le moins possible : les scalaires des gradients conjugués restent sur la carte (un noyau d'un seul fil fait les sommes finales, dans un ordre fixe, et calcule alpha et beta), le CPU ne regarde que toutes les deux itérations si c'est fini, et seules les lignes qui tiennent le système montent et travaillent (un aller-retour coûte plus qu'un cycle entier). Une résolution donne le même résultat d'une fois sur l'autre, et le même que le CPU à la tolérance près (quelques pascals sur une cuve de 960 × 200, pas au bit près). La carte ne sert que sur demande : `AWORLD_GPU=auto` lui envoie les grands systèmes (40 000 inconnues et plus, `AWORLD_GPU_MIN` pour changer le seuil : une poignée de flaques ne vaut pas le voyage), `AWORLD_GPU=1` tout. Sur une carte intégrée (Intel Arc B390), ce n'est pas un gain : une résolution du grand bassin 960 × 200 y prend 22 ms contre 31 sur le CPU, et en jeu, où une résolution tient en quelques itérations, le voyage coûte plus qu'il ne rapporte. Ce n'est pas le calcul qui coûte (un balayage du monde entier y prend 64 µs) mais la centaine de lancements de noyaux dépendants d'un cycle en V, dix à trente microsecondes chacun ; `AWORLD_GPU=1 cargo test --release --bin aworld bench_gpu -- --ignored --nocapture` mesure tout cela sur la carte de la machine (`AWORLD_BENCH_GRID=1920x1080` pour un monde plus grand, où la carte s'amortit mieux). La carte est celle qui dessine (ouverte avec la fenêtre, voir « Le rendu »). La version web résout sur le CPU : attendre le retour de la carte, comme le fait chaque résolution, est impossible dans une page. Le classement, les forces et le transport des liquides restent sur le CPU pour l'instant. Les niveaux grossiers du cycle en V (du niveau 2 au plus grossier, `AWORLD_GPU_FUSE` pour partir d'un autre) tournent en un seul lancement d'un seul groupe de travail (`vcycle_small`) : balayages, résidu restreint, niveau le plus grossier, correction prolongée et balayages, les fils s'attendant entre deux passes ; ces niveaux sont si petits (18 000 cellules et moins) que les trente lancements d'un cycle y coûtaient plus que le calcul (1,5 ms par pas sur l'Arc). Pour cela, les grilles de tous les niveaux grossiers tiennent dans une grille partagée de chaque sorte, chaque niveau à un décalage que la carte accepte pour une liaison.

**Deux bassins qui ne se touchent pas ne se parlent plus** : deux briques lâchées dans un lac faisaient jaillir l'eau du lac voisin, de l'autre côté d'une colline (Thomas : « les masses à côté ne devraient pas réagir »). Le multigrille du solveur de pression regroupe les cellules quatre par quatre, niveau après niveau, et une cellule grossière posée à cheval sur une crête fine réunissait les deux lacs : le préconditionneur faisait passer la correction de l'un dans l'autre, et la résolution, arrêtée à une tolérance relative à l'éclaboussure, laissait le lac voisin loin de son équilibre. Une cellule grossière ne représente plus que les cellules fines qui se tiennent par les arêtes à l'intérieur d'elle (le plus grand groupe ; les autres ne reçoivent ni ne donnent de correction, les balayages s'en chargent), et une arête grossière ne compte que les arêtes fines entre ces cellules : chaque cellule grossière reste dans un seul corps de liquide. Sur le CPU (`Level::parts` dans `mgpcg.rs`) comme sur la carte (les bits 4 à 7 du genre des cellules grossières, `mgpcg.wgsl`). Dans deux cuves séparées par un mur placé en travers des cellules grossières, une éclaboussure dans la cuve de gauche déplaçait la pression de celle de droite de 3,2 MPa ; de 5 Pa au plus maintenant (tests `a_splash_in_one_tank_leaves_the_tank_beside_it_alone`, sur le CPU et sur la carte).

**Tout le pas des liquides sur la carte, en avance d'un pas** (`src/world/liquid_gpu.rs`, noyaux `src/world/liquid.wgsl`) : avec `AWORLD_GPU=1` (ou `auto`), le classement, les forces, la viscosité, la pression, les gouttes jetées, le transport et le débordement sont des noyaux sur la carte, et les grilles y restent d'un pas à l'autre ; le CPU ne lui envoie que les cellules que le reste du jeu a changées et ne reçoit que celles que la carte a changées (listes, pas grilles). Une cellule n'est renvoyée à la carte que si sa matière ou son remplissage a changé, ou si sa chaleur s'est écartée de plus de `TEMP_GAP` (0,5 °C) ou sa pression de gaz de plus de `GASPA_GAP` (20 Pa) de ce que la carte tient : un monde qui tiédit n'envoie plus chaque cellule à chaque pas (95 000 cellules par pas tombent à quelques milliers). Dans l'autre sens, la carte marque chaque cellule qu'elle touche du numéro du pas (`mark`) et le CPU relit, par bandes, la matière, le remplissage, la chaleur et les marques des seules lignes des zones qui tournent, puis parcourt les lignes en parallèle, ce qui a remplacé une liste d'enregistrements et son compteur atomique. La carte travaille pendant que le CPU fait le reste du pas : le pas liquide est soumis après le tour des cellules et son résultat repris au début du pas suivant, avant le tour des cellules, de sorte que la pluie des cellules et la carte ne déplacent jamais la même eau. Ce que la carte a déplacé (remplissage, chaleur) s'ajoute à ce que le jeu a entre-temps (la lave qui refroidit, la surface qui sèche) au lieu de le remplacer ; une cellule dont le jeu a changé la matière garde la version du jeu, la carte en est informée au pas suivant. Le nombre d'itérations de la pression est fixé d'avance pour chaque pas, d'après le pas précédent (une de plus s'il n'avait pas convergé), pour ne jamais attendre la carte au milieu du pas. Sur l'Arc B390, la version synchrone faisait 37,7 ms par pas contre 30,5 sur le CPU (les noyaux hors pression 3 ms contre 16, mais la pression 14,6 ms d'itérations et la carte freine le CPU quand elle travaille) ; la version asynchrone fait 20,9 ms par pas (envoi 5,7, noyaux et attente 4,6, retour 2,5), l'envoi avant l'économie des différences. Si le programme de la carte ne compile pas, l'erreur est imprimée et les liquides restent sur le CPU.

**Amortissement à la surface libre et convergence** : une arête de la surface libre (entre liquide plein et air ou film, entre film et air) perd `CALM` = 3 parts de sa vitesse par seconde, avant la gravité, de sorte qu'une surface au repos le reste ; le tube en U se calme en une quinzaine de secondes au lieu de ballotter sans fin, et l'éclaboussure monte encore. Deux défauts du transport corrigés en passant, trouvés avec les sondes `probe_splash` et `probe_froth` : un film couché sur une cellule qui n'est pas pleine n'y entrait jamais (le film ne porte pas de pression, l'arête sous lui s'arrête), et la mare gardait un millier de cellules jamais pleines et deux cents cellules de film qui ne se posaient pas ; le liquide posé sur une cellule du même liquide qui n'est pas pleine y descend maintenant à chaque sous-pas, de haut en bas. Et les miettes de quelques unités (sur 65 536) restaient en l'air pour toujours, ce qu'une arête fait passer s'arrondissant à zéro ; une cellule qui garde `CRUMB` = 8 unités ou moins rejoint le même liquide dessous ou dessus, ou disparaît s'il n'y en a pas, et une cellule ne déborde que si elle dépasse le plein de plus d'une miette. Le ciel étant logarithmique, une goutte ou une gerbe qui monte avec toute l'énergie d'une cellule comprimée irait bien trop haut : la vitesse de lancement d'une goutte et la vitesse d'une arête qui remonte sont bornées à `LAUNCH` = 4,43 m/s, soit un mètre de montée. L'eau qui sèche (quarts de cellule évaporés, films qui s'évaporent) est comptée dans `World::dried`, de sorte que les tests de volume vérifient eau + vapeur. La poussée de pression de l'air (`pressure_push`) ne déplace plus les liquides physiques : leur pas les pousse déjà par la pression des poches, et une cellule déplacée dans son dos pendant que la carte travaille était comptée deux fois ou perdue (le bilan `AWORLD_DEBUG_MASS` détaille maintenant, à la reprise, ce que la carte a déplacé et jeté, et les cellules changées entre-temps). L'amortissement de la surface libre ne retient plus une surface qui tombe (du liquide sur de l'air, ou sous de l'air avec de l'air dessous) : il tenait un corps d'eau en chute à `CALM`-ième de g, trois mètres par seconde, sans accélération visible ; un bloc lâché tombe maintenant à g (test `water_falls_at_g`). Les gouttes en vol subissent une traînée quadratique vers une vitesse limite de `DROP_TERMINAL_MS` = 9 m/s. Une bulle de gaz prise dans un liquide (la vapeur d'une explosion) remonte par la poussée d'Archimède, une cellule tous les `BUBBLE_RISE` = 10 pas, droit ou de biais, le liquide prenant sa place (les liquides physiques traitent une cellule de gaz comme une paroi, la bulle restait enfermée ; test `a_bubble_rises_through_a_pool`). Un gaz enfermé n'est de l'air ouvert pour le solveur (à la pression de sa poche, ce qui pousse l'eau d'une chaudière) que si sa poche presse le liquide voisin au moins aussi fort que lui (`World::gas_open`) : une bulle que l'eau presse plus fort est une paroi, car le transport ne verse jamais de liquide dans un gaz, et la pression l'y aspirait sans fin, entassant l'eau dans la cellule voisine jusqu'à soixante cellules dans une seule, perdues à la surface. L'eau posée sur un gaz qui n'est pas de l'air ouvert à la pression de sa poche (un nuage, de la fumée) tombe à travers comme la pluie des cellules, seule ou non : pour le solveur ce gaz est une paroi, et deux gouttes de pluie côte à côte (plus des gouttes seules) restaient posées sur le nuage. Avec les liquides sur la carte, le passage à travers la matière mince (un tronc, une couronne : `seep_through_thin`) se fait sur les cellules que la carte vient de rendre, avant que celles du pas ne lui soient envoyées ; il se faisait après l'envoi, la carte tenait pour vides les cellules où l'eau venait de passer, et l'eau qu'elle y faisait couler le pas suivant était perdue à la reprise (une mare à flanc de colline perdait la moitié de son eau derrière un tronc ; test `a_pool_on_a_hillside_runs_through_a_tree`). Le monde est ouvert sur ses côtés : le liquide qui atteint la première ou la dernière colonne en sort, comme une goutte jetée hors de l'écran, et ce qui est parti est compté dans `World::gone`. Les groupes de liaison des noyaux sont fixés une fois par passe et non à chaque lancement (tous les noyaux partagent leur disposition), ce qui était la plus grosse part de l'encodage des commandes. `AWORLD_DEBUG_CARD=1` imprime toutes les cent pas le temps de chaque étape de l'échange avec la carte et, quand la carte offre les horodatages (Vulkan, Metal), le temps que la carte elle-même passe dans chaque phase du pas (envoi, forces, viscosité, pression, transport, copies de retour) avec le nombre de lignes, de sous-pas et d'itérations, `AWORLD_DEBUG_MASS=1` un bilan de l'eau à chaque phase du pas. Au rendu, une cellule d'eau qui n'est pas pleine est un mélange d'eau et d'air : d'autant plus claire et pâle qu'elle est vide. La vue « Simulation » (`--view simulation`) montre les classes des cellules, les zones qui tournent, les vitesses sur la grille, les gouttes en vol et les lignes que la carte traite.

**Matière qui ne retient que par le dessus** (`spec::Holds`) : chaque matière dit dans quels sens elle retient ce qui avance vers elle. Presque toutes retiennent dans tous les sens (`Holds::Every`) ; les arbres (tronc, branches, feuilles, jeune pousse), la cendre, le charbon de bois et les feuilles mortes ne retiennent que ce qui vient du dessus (`Holds::FromAbove`) : un arbre est dans un plan à lui, la cendre et le charbon sont des flocons légers et ouverts que l'eau traverse et soulève. Le petit garçon et les voitures passent au travers et se tiennent dessus, la pluie s'y pose, mais un liquide la traverse de côté (`seep_through_thin`) : chaque ligne où le liquide touche une suite de matière mince (au plus `THIN_RUN` cellules) relie les deux colonnes de part et d'autre comme des vases communicants. Le niveau de chaque colonne est celui de tout ce qu'elle tient, tassé (une cellule au fond d'un lac n'est presque jamais tout à fait pleine, un film qui coule le long d'un tronc ne compte que pour le peu qu'il tient), et la moitié de l'écart passe à chaque pas, une cellule au plus par ligne, du haut de la colonne la plus haute vers l'autre : en jet de gouttes à la vitesse de Torricelli dans une cellule d'air, ou dans le liquide d'en face avec cette vitesse sur ses arêtes. L'eau passait autrefois par la surface seulement et à vitesse nulle : une crue derrière un arbre en laissait passer le quart en douze secondes, elle se met maintenant à niveau (test `a_flood_levels_through_a_tree`). Les grains plus légers que le liquide qui les touche n'ont plus d'inertie dans le liquide (`World::float_on`) : l'écart de pression du liquide entre leurs deux côtés (`side_push`, un niveau plus haut d'un côté, le courant qui les heurte) les pousse, une cellule par poids d'une cellule de liquide dans l'écart ; tenus sous la surface par le tas qui flotte au-dessus, ils montent en biais ou échangent leur place avec le liquide d'à côté, et un tas posé sur l'eau s'étale sur la surface : un tas qui flotte devient un radeau au lieu d'un mur. Un mur de cendre retenait toute l'eau d'une cuve, il en laisse passer plus de la moitié en cinq secondes (test `a_wall_of_ash_or_charcoal_does_not_dam_the_water`).

**Un corps d'eau tombe entier, à g, et les liquides suivent les ticks** : un bloc d'eau lâché dans l'air tombait à un mètre par seconde, en pile de films, et perdait la moitié de son eau en gouttes (Thomas : « l'eau liquide ne tombe pas à la même vitesse que les particules »). Quatre causes, dans l'ordre où elles sont apparues (sonde `falling_block_probe`, qui imprime le haut, le bas et le centre de masse du bloc chaque dixième de seconde, et sa colonne centrale) : (1) son bas mouillait la cellule du dessous de quelques centièmes, puis la règle qui fait couler un liquide posé sur une cellule du même liquide pas pleine (pour qu'un film à la surface d'une mare rejoigne le premier creux) y faisait couler la cellule entière, une cellule par sous-pas, dix mètres par seconde, et le trou remontait à travers le corps : un damier de cellules à 10 % et à 95 %, que le solveur voyait comme autant de surfaces libres et de films, et dont les cellules « non tenues » partaient en gouttes. Le liquide ne coule plus que dans une cellule courte qui est tenue (sur une paroi, sur autre chose, ou sur le même liquide au moins à moitié plein), pas dans celle que sa propre chute vient de mouiller ; un filet de moins d'un quart de cellule coule quand même dans le filet du dessous, sinon deux filets suspendus en l'air y resteraient à jamais, trop minces pour bouger. (2) Une arête dans l'air (de l'air des deux côtés) se lit, pour le transport des vitesses, à la vitesse de l'arête transportée : l'air ne porte pas de quantité de mouvement, et le haut d'un corps qui tombe lisait des zéros au-dessus de lui. (3) L'arête du bas d'une cellule que le liquide remplit par le dessus (son arête du haut est celle qui l'alimente le plus), si elle est dans l'air, prend la vitesse du liquide qui entre dès que la cellule tient un quart : une arête dans l'air est au repos, le front d'un corps qui tombe lisait son propre repos à chaque cellule mouillée, et la pression transmettait ce frein à tout le corps, qui tombait aux quatre cinquièmes de g en ralentissant. (Pas avant le quart : un filet mouillant la cellule suivante à la vitesse du corps, et celle-ci la suivante, courait devant le corps en traînée de films qui partait en gouttes. Pas sur les côtés non plus : passée aux arêtes latérales, la vitesse d'une vague qui mouille sa voisine pompait les vagues du vent, qui doublaient sur la carte. Sur la carte, c'est la cellule du dessous qui écrit son arête du haut : la cellule qui écrivait son arête du bas faisait la course avec ses voisines, et le lac, la bulle et la glace dans le ciel sortaient autrement d'une exécution à l'autre.) (4) L'amortissement de la surface libre (`CALM`) ne s'applique plus à l'arête entre le corps et sa cellule de front (du liquide sur une cellule qui a de l'air sous elle), qui le freinait aussi. Le bloc tombe maintenant entier et à g pendant la première seconde (centre de masse à 1 % de la chute libre, 85 % de l'eau encore en cellules), puis se défait progressivement en gouttes, comme une chute d'eau (test `a_block_of_water_falls_whole_near_g`). Avec le pas de temps variable (`World::step_dt`), les liquides suivent les ticks des cellules : un tour de 1/60 s par tick (`liquid::turns`), un pas de 1/30 s en fait deux à la suite (mêmes sous-pas, mêmes résolutions de pression), un pas plus court qu'un tick les laisse où ils sont, comme les cellules. Résolus une fois par pas quel qu'il soit, ils étaient deux fois plus raides à 1/120 s et deux fois moins à 1/30 s (la pression ne rend le liquide incompressible que là où elle est résolue, le recomblement corrige une part de l'erreur à chaque résolution, et une cellule jette une part d'elle-même en gouttes à chaque sous-pas) : l'eau en cellules de la scène « liquides » après 3 s valait 2,61, 2,32 et 2,20 milliards d'unités à 1/120, 1/60 et 1/30 s ; elle vaut 2,88, 2,87 et 2,84 (`water_over_time_probe`), le reste de l'écart avec les 3,11 du départ étant les gouttes en vol et l'eau qui sort du monde (les blocs lâchés dans le ciel de la scène ne se défont plus en gouttes, d'où le gros de la différence avec avant).

**L'eau ne flotte plus sur un nuage** : de l'eau peinte sur un gros nuage y restait (Thomas : « les liquides continuent à flotter sur la vapeur »), et le monde ralentissait à 87 ms le pas, la couche « Liquides du solveur » montrant 19 000 cellules pleines dans le nuage. Deux causes (sonde `cloud_probe` dans `bench.rs`, qui refait la scène et imprime où va l'eau et ce que coûte le pas, et `slab_probe`, qui suit une petite dalle d'eau sur un nuage tick par tick) : (1) une goutte de pluie qui traverse un nuage en balaie les gouttelettes et grossit (`VAPOUR_PER_WATER` cases de nuage, une cellule d'eau), et cette cellule naissait sous la goutte, qui restait posée dessus : de l'eau sur de l'eau est au solveur, qui ne pousse pas d'eau dans un gaz, et seule la cellule du bas pleuvait, en balayant la suivante ; chaque goutte laissait derrière elle une colonne d'eau debout dans le nuage, des milliers de cellules que le solveur travaillait et qui se vidaient par le bas d'une cellule par tick. L'eau balayée part maintenant en goutte volante (celles du solveur, qui traversent les gaz) à la vitesse de la pluie, et la goutte continue de tomber. (2) Un gaz en plein ciel (un nuage, la vapeur au-dessus d'un bain) comptait comme de l'air pour les liquides, à la pression du ciel, tant que l'eau posée dessus ne pesait pas plus : le solveur la poussait vers le nuage sans pouvoir l'y mettre, et elle ne tombait pas en pluie non plus (il n'y a pluie que sur un gaz qui est une paroi pour les liquides). Seul un gaz enfermé dans une poche (une chaudière, une bulle) est de l'air pour eux, à la pression de la poche, comme le disait déjà le commentaire de `gas_open`. (L'eau posée sur le nuage tombe ensuite par sa rangée du bas, chaque rangée partant quand celle du dessous est partie, à une douzaine de rangées par seconde : faire pleuvoir toute la colonne d'un coup, essayé, perdait au solveur la colonne d'un lac au-dessus d'une bulle, et un bloc de glace y basculait et coulait.) Au passage, un film d'eau (moins d'une demi-cellule) ne retient plus un bloc qui tombe : pris dans un creux d'un bloc de glace qui se défaisait, il le tenait en l'air (`piece_can_move`). Le coût du pas sur cette scène, sur le CPU, était de 44 ms avant #104 et de 28 ms sur main : le code n'a pas ralenti, c'est la scène qui s'était alourdie des colonnes ; il est à 20 ms avec ces correctifs.

**Les points chauds au flame graph** : `cargo test --release --bin aworld flame_probe -- --ignored --nocapture` (avec `CARGO_PROFILE_RELEASE_DEBUG=1` pour les noms des fonctions en ligne) échantillonne chaque scène de `bench_scenes` à 500 Hz sur tous les fils (`pprof`, en dépendance de test seulement) et écrit `flame-<scène>.svg` (le graphe, à ouvrir dans un navigateur), `.txt` (les piles les plus lourdes) et `.folded` (les piles pliées, pour tout autre outil) dans `AWORLD_FLAME_DIR` ; `perf` manquait sur la machine de mesure, et le jeu n'a pas d'exécutif asynchrone (la boucle de la fenêtre et rayon, pas tokio), donc pas de console tokio. Ce que les graphes ont montré, et ce qui en a été fait : (1) la largeur du monde n'est connue qu'à l'exécution depuis `--grid`, et `i / W()`, `i % W()` étaient de vraies divisions, que le tour des cellules faisait pour la plupart des cellules touchées (`Zones::wake` à 4,5 % de la scène « liquides », `div_ceil` 4,7 %) : `xy(i)` les fait par une multiplication et un décalage (Granlund et Montgomery, exact jusqu'à 2³¹ cellules, vérifié par un test pour toutes les largeurs), les zones gardent leur compte, et `W()` et `H()` sont des lectures ordinaires que le compilateur peut sortir des boucles (le tableau n'est écrit qu'une fois, avant tout monde) ; `wake` ne regarde les zones voisines que pour une cellule au bord de la sienne. (2) `step_cloud` regardait les 24 cases autour de chaque case de nuage à chaque tick (10 % de la scène « nuage ») pour un tirage à une chance sur 8 000 : le tirage vient d'abord, le regard après. (3) Le débordement des liquides (deux passes en série sur toutes les travées, 10 % de « liquides ») cherche les cellules qui peuvent bouger par un balayage serré des remplissages (une miette, ou une cellule sur une cellule pas pleine) et ne lit le reste que là ; ce qu'il lui reste, c'est les 12 000 cellules qui coulent réellement d'une cellule pas pleine à chaque sous-pas de cette scène. Au total, 5 à 16 % d'échantillons CPU en moins selon la scène (démo 9 %, explosions 16 %, liquides 10 %, orage 6 %, incendie 5 %, nuage 6 %, sur 300 pas) ; le gros du pas des liquides reste la pression (10 ms sur 40, 2,6 ms l'itération du multigrille), les forces (9) et le transport (11, dont 2,4 ms de passes parallèles par sous-pas), tous déjà sur tous les cœurs, et un masque des arêtes dans l'air pour les quatre nœuds d'un échantillon, essayé, ne changeait rien. Le temps de mur sur la machine de mesure varie de 5 % d'une exécution à l'autre : comparer les échantillons CPU des `.folded` (leur somme) plutôt que les ms par pas.

**La vue « Simulation »** (`src/simview.rs`, lectures dans `src/world/inspect.rs`) montre le monde tel que le calcul le tient. Un panneau flottant, qu'on déplace par son titre, choisit ce que montre le fond, en mots simples : la matière ; la pression qui pousse sur les parois (dans les liquides, par leur physique ou par le compte des cellules, et dans les poches de gaz fermées), en Pa au-dessus de l'air libre ; la pression de l'air (`flow.rs`) ; les changements du dernier pas (matière, liquide, chaleur, pression, vitesse) ; et, quand les liquides suivent leur physique, leur calcul (plein, film, goutte). Les pressions sont sur une échelle logarithmique, pour qu'une mare (quelques kPa) et une poche fermée (des centaines) se voient ensemble ; les changements, à l'échelle de 99 cases sur 100 (au-delà de dix fois, en magenta). Par-dessus : les flèches de vitesse de chaque type de matière, chacune lue dans le modèle qui la porte, en m/s (1 m/s = une case de flèche) : la grille des liquides physiques ; la vitesse de chute des grains et des gouttes (`vel` : un grain qui glisse sur un tas n'a pas de vitesse, et une goutte qui rejoint l'eau perd la sienne) ; la vitesse de côté d'une goutte sortie d'un jet sous pression (`jet`) ; le flux de l'air pour les gaz, les flammes et l'air ; la chute des morceaux et le corps rigide des dalles pour les solides. La marque au pied de la flèche dit laquelle. La couche « Liquides du solveur », quand les liquides suivent leur physique, colore par-dessus le monde ce que le solveur tient : bleu pour une cellule pleine, bleu pâle pour un film, jaune pour une goutte laissée au tour des cellules, et orange pour une case de gaz voisine du liquide que le solveur prend pour une paroi (l'eau s'y pose comme sur du solide) ; le panneau en donne les comptes. On peut aussi afficher les zones de calcul actives, les particules en vol avec leur vitesse (gouttes, éclats des cratères, débris des voitures, fusées et étincelles) et les lignes que calcule la carte graphique. Sous la souris, le panneau lit la case : matière, remplissage, vitesse et sa source, pressions, poche de gaz. **Pas à pas** : « 1 pas », « 10 pas » ou la touche point mettent le monde en pause après avoir gardé l'état de chaque case ; le fond « changements » montre ce que ces pas ont modifié, et le panneau donne la valeur d'avant de la case pointée. Avec les liquides sur la carte, un pas reprend le résultat liquide du pas précédent : le diff montre le pas liquide d'avant. Pour les captures : `--sim-back` (`matiere`, `pression`, `air`, `calcul`, `diff-remplissage`, `diff-temperature`, `diff-pression`, `diff-vitesse`), `--sim-steps n`, `--pour x`, `--steam x` (une couche de vapeur sous une couche d'eau).

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
- **Poussée d'Archimède** : un grain plus léger que le liquide qui l'entoure remonte (charbon de bois, cendre, feuilles mortes, grêle flottent sur l'eau), un plus lourd coule. Un bloc qui tombe fait pareil selon sa densité moyenne : une rondelle de bois flotte comme la glace, un rocher coule au fond du lac en poussant l'eau (avant, il restait posé sur l'eau). Une pièce entrée sous l'eau d'un plongeon (une bûche lâchée de haut) et plus légère qu'elle remonte d'une ligne par pas jusqu'à sortir le dessus (`World::buoyed` : de l'eau en corps au-dessus, de l'eau dessous) ; elle restait coincée, ne pouvant ni descendre ni monter à travers l'eau.

## Dessin : ne recolorier que ce qui change

- Avant, chaque image recalculait la couleur des 518 400 cellules (environ 3 ms sur un cœur) et rebalayait tout le monde pour retrouver les arbres (0,5 ms). Maintenant le dessin ne recolorie que les zones de 32×32 qui ont tourné ou ont été touchées depuis la dernière image (`Zones::changed_since`), plus une rangée de zones à tour de rôle pour les changements lents qui ne réveillent pas une zone (teinte de la chaleur, humidité du sol) : tout est rafraîchi au moins toutes les 17 images. Sur la campagne, une soixantaine de zones sur 510 par image, soit environ 10 fois moins de travail. Les arbres et les nuages suivent la même règle, zone par zone.
- Le scintillement de l'eau et des braises, de la lave et des flammes est passé dans le shader (une graine de bruit par cellule et le temps) : une cellule qui ne change pas garde exactement la même couleur dans la texture, ce qui rend le recoloriage partiel exact.

## Capteurs : thermomètre, baromètre, hygromètre

Le nouvel outil **Capteur** (barre d'outils à gauche, icône thermomètre) pose une sonde dans le monde, jusqu'à 4 (A, B, C, D). Chaque sonde relève toutes les 30 étapes (une demi-seconde de jeu) la température, la pression absolue (celle de la poche de gaz, ou dans un liquide le poids de ce qui est au-dessus plus le gaz qui appuie) et l'humidité de l'air. Ses courbes des deux dernières minutes s'affichent dans une carte en bas de l'image, chaque courbe à sa propre échelle pour qu'un petit changement se voie. On y voit l'eau d'une casserole chauffer sans jamais dépasser 100 °C, une chaudière monter en pression, l'air sécher à midi. Cliquer sur une sonde l'enlève ; « Réessayer » efface les courbes mais garde les sondes. Les sondes ne sont pas enregistrées dans les sauvegardes.

## Lumière et effets (shaders)

- **Reflets** : la surface de l'eau scintille au soleil (paillettes plus nombreuses sous le soleil), et à la lune la nuit.
- **Lumière du feu** : la nuit, le feu, les braises et la lave éclairent tout autour d'eux d'une lumière chaude qui vacille.
- **Flammes** : au-dessus du feu dansent des langues de flamme, jaunes à la base, orange puis rouges aux pointes. Elles ondulent, s'étirent et se déchirent en petites flammèches qui montent. La lave et les braises rougeoient sans faire de flammes.
- **Air chaud** : au-dessus d'un feu ou de la lave, l'air chaud fait onduler ce qu'on voit à travers (comme au-dessus d'une route en été).
- **Aube et crépuscule** : quand le soleil est bas, le ciel passe à l'orange près de l'horizon, au rose au-dessus et au violet en haut, avec un halo doré du côté où il se lève ou se couche. Le ciel ne s'éteint pas d'un coup : la lueur reste un moment après le coucher. Les nuages sont éclairés par en dessous et sur la face tournée vers le soleil, en or près de lui et en rose plus loin, leur sommet dans une ombre lavande.
- Une petite carte de lumière (une case pour 6×6 cellules) suffit : tout reste fluide.

**Le rendu** (`src/gfx/`) : winit ouvre la fenêtre et donne le clavier et la souris, wgpu dessine chaque pixel (Vulkan,
Metal, DirectX 12, WebGPU ou WebGL 2), sur la même carte que celle où calculent les liquides (`world::gpu`, ouverte
avec la fenêtre). Le jeu dessine comme avant par appels immédiats (`draw_circle`, `draw_text`, `draw_texture_ex`...) :
tout ce qu'une image dessine va dans une seule liste de triangles, coupée en appels là où la texture ou le matériau
change et en passes là où la cible change (`set_target`, pour les nuages floutés et le fondu des fenêtres), puis part
à la carte en une fois à la fin de l'image, multiéchantillonnée (4 échantillons par pixel). Les shaders sont en WGSL :
un matériau (`load_material`) donne son `fs_main`, ses uniformes (groupe 2) et sa façon de se mélanger, derrière un
prélude commun (le vertex shader, la projection, la texture dessinée lue par `tex(uv)`). Le texte : la police TrueType
rastérisée une fois par taille dans un atlas (fontdue). Les lettres suivent la disposition du clavier (Z est la touche
marquée Z, en AZERTY aussi) ; les chiffres sont ceux de la rangée du haut.

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

## Bûches et arbres (palette Bois)

- **Une bûche par coup de pinceau** : le bois peint d'un trait est un corps rigide à lui seul (`World::log`, un numéro de bûche par cellule). Il ne se colle ni au sol, ni à une falaise, ni à une autre bûche : il repose sur ce qui est dessous et porte ce qui est posé dessus. Une bûche peinte en l'air ou contre une paroi tombe donc d'un bloc, garde sa forme et s'empile sur les autres au lieu de se fondre dans le terrain.
- Une pièce qui tombe sur du sable ou de la terre tassés y reste posée si elle est plus légère (Archimède, comme sur l'eau) : une bûche ne s'enfonce plus dans une dune, la roche si.
- **Arbre** : la petite flèche du bouton Bois propose « Arbre ». Un clic plante un arbre adulte (tronc, branches, feuillage, comme ceux du paysage) sur le sol sous la souris ; un pinceau plus gros le fait plus grand. Pas d'arbre dans l'eau ni sous un surplomb.

`--demo out.png --logs --tab matter --states 4` montre des arbres plantés et des bûches tombées en pile.

## Construction, test et défi
Le jeu sépare maintenant clairement les trois moments, affichés dans la pastille en haut à droite :
- **CONSTRUCTION** : rien ne bouge ; tout ce qu'on fait modifie le monde construit (le « plan »).
- **TEST** (bouton Tester ou Espace) : le plan est mis de côté au premier lancement, puis on regarde. On peut encore bricoler, même en pause, mais rien n'est gardé : Pause puis Reprendre continue le même test, et Annuler s'arrête au plan.
- **DÉFI** (bouton Défi) : repart du plan, compteurs Arrivées/Perdues à zéro, et verrouille la construction (les voitures, les capteurs, le lance-flammes et l'éclair restent permis).

**Réessayer** (R, dans la barre du lecteur et en bas de la palette) remet le plan, remet les compteurs à zéro et revient à la construction ; en construction, il ne fait rien. Une sauvegarde faite pendant un test ou un défi enregistre le plan, jamais un monde à moitié simulé.

`--demo out.png --challenge` montre le défi en cours.
