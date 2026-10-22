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

## Compiler et diffuser

- `./build.sh` (Linux) ou `build.bat` (Windows) : l'application de bureau, `target/release/aworld`.
- `./build.sh web` ou `build.bat web` : le paquet web dans `dist/web/` (wasm + page + petit serveur
  `aworld-serve`). Lancer `dist/web/serve.sh` (ou `serve.bat`) puis ouvrir http://localhost:8765/ ;
  `./build.sh web run` fait les deux. Le dossier `dist/web/site/` peut aussi être posé sur n'importe quel hébergeur statique.
- Version web : les sauvegardes restent en mémoire tant que la page est ouverte (pas de fichiers).
- Prérequis web : `rustup target add wasm32-unknown-unknown` (le script le tente).

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
- Câbles : un bout peut rester libre (le second clic dans le vide) ; l'acier est bien plus raide.

## L'air qui s'écoule

Le vent est un écoulement simplifié (grille grossière, fluide incompressible) : des couches d'air soufflent en sens
opposés selon l'altitude, des rafales s'enroulent en tourbillons, l'air chaud monte au-dessus d'un feu, et
l'écoulement contourne le relief. Nuages, vapeur et fumée le suivent, la pluie se penche avec le vent local.
Touche **V** pour voir les flèches.
