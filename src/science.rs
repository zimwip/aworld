//! The educational side: what each material is, and the recipes that link them.

use crate::world::Mat;
use macroquad::prelude::Color;

pub struct Info {
    pub name: &'static str,
    pub state: &'static str,
    pub fact: &'static str,
}

pub fn info(m: Mat) -> Info {
    let (name, state, fact) = match m {
        Mat::Empty => ("Air", "gaz", "L'air est plus froid en altitude et la nuit."),
        Mat::Sand => ("Sable", "poudre", "Chauffé à 600 °C, il fond et devient du verre."),
        Mat::Water => ("Eau", "liquide", "Bout à 100 °C à l'air libre, plus chaud sous pression (120 °C à 2 atm : la cocotte-minute). Gèle sous -6 °C."),
        Mat::Fire => ("Feu", "flamme", "Environ 650 °C. Il brûle le bois et chauffe tout autour."),
        Mat::Steam => ("Vapeur", "gaz", "Eau chauffée à plus de 100 °C : elle prend bien plus de place que l'eau. Enfermée, elle pousse (chaudière, moteur à vapeur)."),
        Mat::Iron => ("Fer", "solide", "Rouille dans l'eau. Avec du carbone et de la chaleur, il devient de l'acier."),
        Mat::Wood => ("Bois", "solide", "Brûle vers 300 °C. Il reste de la cendre, c'est du carbone."),
        Mat::Stone => ("Pierre", "solide", "Fond vers 540 °C. Le gel et le dégel l'effritent en sable."),
        Mat::Brick => ("Brique", "solide", "Brique de plastique emboîtable."),
        Mat::Clay => ("Argile", "solide", "Terre grasse. Cuite dans un feu vers 550 °C, elle devient de la terre cuite."),
        Mat::Terracotta => ("Terre cuite", "solide", "Argile cuite : elle tient la chaleur et ne brûle pas. Idéale pour un creuset sous un feu de bois."),
        Mat::Rust => ("Rouille", "poudre", "Fer abîmé par l'eau. Chauffée dans un feu, elle redonne du fer."),
        Mat::Ash => ("Cendre", "poudre", "Du carbone. Mélangée à du fer chaud, elle fait de l'acier ; avec l'eau, de la terre."),
        Mat::Ember => ("Braise", "solide", "Du bois qui brûle. Il devient de la cendre."),
        Mat::Lava => ("Lave", "liquide", "Plus de 1000 °C. Elle refroidit très lentement et devient de la pierre."),
        Mat::Ice => ("Glace", "solide", "Eau gelée. Elle fond dès qu'il fait plus de 3 °C."),
        Mat::Gunpowder => ("Poudre", "poudre", "Explose dès 210 °C. Attention au feu !"),
        Mat::Dirt => ("Terre", "solide", "Boit la pluie. Avec une graine, de l'eau et de la lumière : un arbre."),
        Mat::Grass => ("Herbe", "solide", "Ses racines retiennent la terre contre l'érosion."),
        Mat::Leaf => ("Feuille", "solide", "Verte au printemps, rousse en automne, puis elle tombe."),
        Mat::Steel => ("Acier", "solide", "Fer + carbone : bien plus solide, et il rouille moins vite."),
        Mat::Cloud => ("Nuage", "gaz", "Vapeur refroidie. Quand il est épais, il pleut (ou il neige s'il gèle)."),
        Mat::Seed => ("Graine", "poudre", "Elle pousse sur de la terre humide, en pleine lumière, si elle a de la place."),
        Mat::Sapling => ("Pousse", "solide", "Elle grandit le jour en buvant de l'eau, jamais la nuit."),
        Mat::Mud => ("Boue", "poudre", "Terre emportée par l'eau. En séchant, elle redevient de la terre."),
        Mat::Glass => ("Verre", "solide", "Sable fondu. Il casse sous un choc ou un choc thermique."),
        Mat::Snow => ("Neige", "poudre", "Nuage qui précipite quand il gèle. Elle fond au-dessus de 3 °C."),
        Mat::Molten => ("Métal fondu", "liquide", "Du fer qui a fondu dans la lave fraîche (plus de 1000 °C). Il se fige en refroidissant."),
        Mat::Oil => ("Pétrole", "liquide", "Du bois enfoui très longtemps. Plus léger que l'eau, il flotte et brûle très fort."),
        Mat::Smoke => ("Fumée", "gaz", "Produite par le feu et les explosions. Elle monte, le vent la pousse, elle se dilue."),
        Mat::Hail => ("Grêle", "poudre", "Petits grains de glace : il tombe de la glace quand le nuage est très froid."),
        Mat::Shard => ("Éclats", "poudre", "Verre brisé. Avec le temps, il redevient du sable."),
        Mat::DeadLeaf => ("Feuille morte", "poudre", "Elle pourrit doucement et devient de la terre."),
        Mat::Branch => ("Branche", "solide", "Le squelette de la couronne : au printemps, les feuilles repoussent."),
        Mat::Trunk => ("Tronc", "solide", "Les voitures passent à travers les arbres. Il brûle comme du bois."),
    };
    Info { name, state, fact }
}

pub struct Recipe {
    /// Left side, shown as coloured swatches.
    pub from: &'static [Mat],
    pub to: &'static [Mat],
    pub text: &'static str,
}

pub const RECIPES: &[Recipe] = &[
    Recipe { from: &[Mat::Clay, Mat::Fire], to: &[Mat::Terracotta], text: "L'argile cuite vers 550 °C devient de la terre cuite : un creuset qui tient la chaleur et ne brûle pas." },
    Recipe { from: &[Mat::Wood, Mat::Fire], to: &[Mat::Ash], text: "Le bois brûle : il reste du carbone (la cendre)." },
    Recipe { from: &[Mat::Iron, Mat::Ash, Mat::Fire], to: &[Mat::Steel], text: "Fer + carbone + chaleur (400 °C) : de l'acier. Dans un creuset de terre cuite sous un feu de bois, avec la cendre prise à la pelle." },
    Recipe { from: &[Mat::Steel, Mat::Water], to: &[Mat::Rust], text: "L'eau rouille le fer, l'acier un peu moins vite." },
    Recipe { from: &[Mat::Rust, Mat::Fire], to: &[Mat::Iron], text: "En chauffant la rouille, on retrouve du fer." },
    Recipe { from: &[Mat::Water, Mat::Fire], to: &[Mat::Steam], text: "L'eau bout à 100 °C et devient vapeur." },
    Recipe { from: &[Mat::Steam], to: &[Mat::Cloud, Mat::Water], text: "La vapeur refroidit : nuage, puis pluie. C'est le cycle de l'eau." },
    Recipe { from: &[Mat::Water], to: &[Mat::Ice], text: "La nuit, quand il gèle, l'eau devient de la glace." },
    Recipe { from: &[Mat::Cloud], to: &[Mat::Water, Mat::Snow, Mat::Hail], text: "Nuage tiède : pluie. Gel : neige. Grand froid : grêle. Un nuage réchauffé par le soleil ne pleut pas." },
    Recipe { from: &[Mat::Sand, Mat::Fire], to: &[Mat::Glass], text: "Du sable très chaud fond : c'est du verre." },
    Recipe { from: &[Mat::Glass], to: &[Mat::Shard, Mat::Sand], text: "Le verre brisé redevient du sable avec le temps." },
    Recipe { from: &[Mat::Stone, Mat::Fire], to: &[Mat::Lava], text: "La roche chauffée longtemps fond en lave." },
    Recipe { from: &[Mat::Lava, Mat::Stone], to: &[Mat::Lava], text: "La lave fraîche est très chaude : la chaleur gagne la roche voisine qui fond à son tour." },
    Recipe { from: &[Mat::Lava, Mat::Iron], to: &[Mat::Molten], text: "Seule la lave fraîche (plus de 1000 °C) fait fondre le métal." },
    Recipe { from: &[Mat::Lava, Mat::Water], to: &[Mat::Stone], text: "La lave arrosée se fige en pierre." },
    Recipe { from: &[Mat::Stone, Mat::Ice], to: &[Mat::Sand], text: "Le gel et le dégel effritent la roche en sable." },
    Recipe { from: &[Mat::Dirt, Mat::Seed, Mat::Water], to: &[Mat::Trunk, Mat::Leaf], text: "Terre + graine + eau + lumière : un arbre pousse." },
    Recipe { from: &[Mat::Wood, Mat::Dirt], to: &[Mat::Oil], text: "Du bois enterré très longtemps devient du pétrole. Il flotte sur l'eau et brûle très fort." },
    Recipe { from: &[Mat::Gunpowder, Mat::Fire], to: &[Mat::Smoke, Mat::Fire], text: "La poudre explose : beaucoup de gaz, une onde de pression qui casse la roche et pousse tout." },
    Recipe { from: &[Mat::Leaf], to: &[Mat::DeadLeaf, Mat::Dirt], text: "En automne les feuilles tombent et deviennent de la terre." },
];

pub fn mat_color(m: Mat) -> Color {
    let (r, g, b) = match m {
        Mat::Sand => (226, 200, 124),
        Mat::Water => (44, 124, 224),
        Mat::Lava => (255, 90, 14),
        Mat::Fire => (255, 150, 30),
        Mat::Steam => (205, 214, 230),
        Mat::Ice => (176, 226, 248),
        Mat::Gunpowder => (66, 62, 74),
        Mat::Iron => (146, 156, 174),
        Mat::Steel => (112, 134, 168),
        Mat::Wood => (150, 100, 54),
        Mat::Trunk => (112, 78, 46),
        Mat::Stone => (110, 110, 122),
        Mat::Dirt => (112, 78, 52),
        Mat::Mud => (88, 62, 42),
        Mat::Grass => (88, 172, 64),
        Mat::Leaf => (60, 150, 64),
        Mat::DeadLeaf => (190, 116, 44),
        Mat::Branch => (104, 70, 40),
        Mat::Seed => (138, 96, 50),
        Mat::Sapling => (110, 206, 84),
        Mat::Ash => (110, 110, 116),
        Mat::Rust => (156, 84, 42),
        Mat::Glass => (190, 226, 246),
        Mat::Shard => (196, 230, 246),
        Mat::Snow => (244, 247, 252),
        Mat::Hail => (210, 236, 252),
        Mat::Oil => (40, 34, 36),
        Mat::Molten => (255, 210, 110),
        Mat::Smoke => (96, 96, 104),
        Mat::Cloud => (246, 248, 253),
        Mat::Ember => (255, 120, 24),
        Mat::Brick => (184, 72, 56),
        Mat::Clay => (168, 124, 98),
        Mat::Terracotta => (196, 98, 60),
        Mat::Empty => (30, 36, 56),
    };
    Color::from_rgba(r, g, b, 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_material_is_documented() {
        for m in Mat::ALL {
            let i = info(m);
            assert!(!i.name.is_empty() && !i.fact.is_empty(), "{m:?}");
        }
    }

    #[test]
    fn recipes_only_use_real_materials() {
        for r in RECIPES {
            assert!(!r.from.is_empty() && !r.to.is_empty() && !r.text.is_empty());
        }
    }
}
