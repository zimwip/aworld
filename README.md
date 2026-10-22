# AWorld

Bac à sable 2D type "falling sand" en Rust (macroquad, rendu GPU). `cargo run --release`.

- Palette en haut (ou touches 1-9,0) : sable, eau, feu, vapeur, fer, bois, pierre, briques/poutres/planches (blocs dérivés), gomme, entrée, sortie.
- Clic gauche dessine, clic droit efface, molette = taille du pinceau, T = tourner un bloc.
- Espace : lancer/pauser la simulation (construction = monde figé). R : revenir au monde d'avant lancement. G : nouveau monde, N : monde vide.
- C : lancer une voiture depuis l'entrée. Elle suit le terrain, saute avec sa vitesse, fait demi-tour devant un obstacle, explose si choc trop fort, disparaît dans le vide, gagne en touchant la sortie.
- Interactions : feu+eau = vapeur (qui retombe en pluie), feu brûle le bois (braises → cendres), l'eau rouille le fer (poudre de rouille qui s'effondre).
