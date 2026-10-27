//! The educational side: what each material is, and the recipes that link them.

use crate::world::Mat;
use crate::gfx::Color;

pub struct Info {
    pub name: &'static str,
    pub state: &'static str,
    pub fact: &'static str,
}

pub fn info(m: Mat) -> Info {
    let (name, state, fact) = match m {
        Mat::Empty => ("Air", "gaz", "L'air est plus froid en altitude et la nuit."),
        Mat::Sand => ("Sable", "poudre", "Chauffé à 600 °C, il fond et devient du verre. Sec, il coule en tas plats (environ 33°) ; mouillé, il tient (châteaux de sable)."),
        Mat::Water => ("Eau", "liquide", "Bout à 100 °C à l'air libre, plus chaud sous pression (120 °C à 2 atm : la cocotte-minute). Gèle à 0 °C. Tant qu'elle bout ou gèle, sa température ne bouge plus : c'est le palier. Elle s'évapore aussi sans bouillir, plus vite au chaud, à l'air sec et au vent."),
        Mat::Fire => ("Feu", "flamme", "Environ 650 °C. Il brûle le bois et chauffe tout autour. Il lui faut de l'air : enfermé, il remplit l'air de fumée et s'étouffe."),
        Mat::Steam => ("Vapeur", "gaz", "Eau chauffée à plus de 100 °C : elle prend bien plus de place que l'eau. Enfermée, elle pousse (chaudière, moteur à vapeur). Refroidie, c'est de la vapeur d'eau : plus légère que l'air sec, elle monte. Arrivée à son point de rosée (plus haut quand elle est dense), elle se condense en nuage et rend la chaleur qui l'avait fait bouillir."),
        Mat::Iron => ("Fer", "solide", "Rouille dans l'eau. Avec du carbone et de la chaleur, il devient de l'acier. Chauffé, il rougit vers 500 °C, jaunit, puis fond vers 900 °C."),
        Mat::Wood => ("Bois", "solide", "Chaque coup de pinceau fait une bûche : un bloc rigide qui tombe d'une pièce et repose sur ce qui est dessous, sans se coller au reste. Brûle vers 300 °C s'il a de l'air, et laisse de la cendre. Chauffé sans air (couvert de terre), il devient du charbon de bois."),
        Mat::Stone => ("Pierre", "solide", "Fond vers 540 °C. Le gel et le dégel l'effritent en sable."),
        Mat::Brick => ("Brique", "solide", "Brique de plastique emboîtable."),
        Mat::Clay => ("Argile", "solide", "Terre grasse. Cuite dans un feu vers 550 °C, elle devient de la terre cuite."),
        Mat::Vault => ("Voûte", "solide", "Maçonnerie posée par l'outil Tunnel au-dessus du trou, ancrée dans le terrain autour : elle tient le plafond. Les vrais tunnels font pareil (voûte en briques, puis béton projeté et boulons dans la roche)."),
        Mat::Terracotta => ("Terre cuite", "solide", "Argile cuite : elle tient la chaleur et ne brûle pas. Idéale pour un creuset sous un feu de bois."),
        Mat::Rust => ("Rouille", "poudre", "Fer abîmé par l'eau (de l'oxyde de fer). Chauffée avec du carbone (charbon de bois, ou dans les flammes d'un feu de bois), elle redonne du fer."),
        Mat::Ash => ("Cendre", "poudre", "Ce qui reste quand le bois a brûlé à l'air : des minéraux, presque plus de carbone. Avec l'eau, elle enrichit la terre."),
        Mat::Ember => ("Braise", "solide", "Du bois ou du charbon qui brûle. Sans air, elle ne propage plus le feu ; elle devient de la cendre."),
        Mat::Lava => ("Lave", "liquide", "Plus de 1000 °C : de la roche fondue. Elle se fige en pierre à 540 °C, mais reste à cette température tant qu'elle n'a pas rendu toute sa chaleur latente : une croûte se forme et garde le cœur liquide. Une coulée qui continue d'arriver fait fondre son lit."),
        Mat::Ice => ("Glace", "solide", "Eau gelée : un bloc solide et lourd qui tombe s'il n'est pas posé sur quelque chose (les feuilles ne le portent pas). Elle fond à 0 °C, mais lentement : il faut beaucoup de chaleur pour la faire fondre."),
        Mat::Gunpowder => ("Poudre", "poudre", "Explose dès 210 °C. Attention au feu ! Elle déflagre : sa pression plafonne vers 3 000 atm."),
        Mat::Dynamite => ("Dynamite", "solide", "Un explosif brisant (nitroglycérine dans de la sciure, Nobel, 1867) : il détone, son onde de choc dépasse 100 000 atm. Assez pour changer le charbon de bois voisin en diamant."),
        Mat::Diamond => ("Diamant", "solide", "Du carbone serré au-delà de 50 000 atm et à plus de 1 500 °C : ses atomes se rangent en cristal. C'est ce qui se passe à 150 km sous terre, ou dans l'onde de choc d'une détonation (les nanodiamants). Le plus dur des matériaux, et le meilleur conducteur de chaleur."),
        Mat::Dirt => ("Terre", "solide", "Elle boit l'eau jusqu'à saturation (trois cellules d'eau), puis l'eau reste en flaque. Mouillée, elle fonce. Son eau descend, remonte doucement vers la surface sèche (capillarité) et repart dans l'air en vapeur. Humide, au soleil, l'herbe y pousse ; semée de graines, elle devient fertile et un arbre y pousse."),
        Mat::Grass => ("Herbe", "solide", "Elle pousse toute seule sur la terre humide, au soleil, et gagne de proche en proche. Quand la terre sèche, elle jaunit puis meurt ; enterrée ou noyée aussi. Ses racines retiennent la terre contre l'érosion."),
        Mat::Leaf => ("Feuille", "solide", "Verte au printemps, rousse en automne, puis elle tombe. Le jour, elle rejette de la vapeur d'eau : l'eau que les racines ont bue (la transpiration)."),
        Mat::Steel => ("Acier", "solide", "Fer + carbone : bien plus solide, et il rouille moins vite."),
        Mat::Cloud => ("Nuage", "gaz", "Vapeur refroidie jusqu'à son point de rosée : de minuscules gouttes. En se formant, il réchauffe l'air (la chaleur de la condensation, le moteur des orages). Quand il est épais, il pleut (ou il neige s'il gèle)."),
        Mat::Seed => ("Graine", "semence", "Semée, elle tombe et se mêle à la terre, qui devient fertile (on voit ses grains clairs). Une terre fertile, humide et au soleil fait pousser un arbre, s'il a de la place. Les arbres en laissent tomber d'autres."),
        Mat::Sapling => ("Pousse", "solide", "Elle grandit le jour en buvant de l'eau, jamais la nuit."),
        Mat::Mud => ("Boue", "poudre", "Terre emportée par l'eau. En séchant, elle redevient de la terre."),
        Mat::Glass => ("Verre", "solide", "Sable fondu. Il casse sous un choc ou un choc thermique."),
        Mat::Snow => ("Neige", "liquide visqueux", "Nuage qui précipite quand il gèle. Elle tombe doucement, puis un tas s'étale lentement, comme une pâte très épaisse ; une couche fine reste en place. Elle fond à 0 °C."),
        Mat::Molten => ("Fer fondu", "liquide", "Du fer chauffé au-delà de 900 °C : dans la lave fraîche, ou au cœur d'une forge au charbon de bois bien fermée. Il coule comme de la lave et se fige en fer en refroidissant ; l'eau le trempe d'un coup."),
        Mat::MoltenSteel => ("Acier fondu", "liquide", "De l'acier chauffé au-delà de 900 °C. Coulé dans un moule de terre cuite, il se fige en acier en refroidissant."),
        Mat::Oil => ("Pétrole", "liquide", "Du bois enfoui très longtemps. Plus léger que l'eau, il flotte et brûle très fort."),
        Mat::Smoke => ("Fumée", "gaz", "Produite par le feu et les explosions. Elle monte, le vent la pousse, elle se dilue à l'air libre. Dans une boîte fermée, elle reste et étouffe le feu."),
        Mat::Hail => ("Grêle", "poudre", "Petits grains de glace : il tombe de la glace quand le nuage est très froid."),
        Mat::Shard => ("Éclats", "poudre", "Verre brisé. Avec le temps, il redevient du sable."),
        Mat::DeadLeaf => ("Feuille morte", "poudre", "Elle pourrit doucement et devient de la terre."),
        Mat::Branch => ("Branche", "solide", "Le squelette de la couronne : au printemps, les feuilles repoussent."),
        Mat::Saltpeter => ("Salpêtre", "poudre", "Des cristaux blancs (du nitrate de potassium) qui poussent là où des feuilles pourrissent avec de la cendre de bois. Mélangé au charbon de bois, à froid et au sec : de la poudre. L'eau le dissout."),
        Mat::Charcoal => ("Charbon de bois", "poudre", "Du bois chauffé sans air : presque du carbone pur. Il brûle lentement et très chaud, et donne son carbone au fer (acier) et à la rouille (fer)."),
        Mat::Trunk => ("Tronc", "solide", "Les voitures passent à travers les arbres. Il brûle comme du bois. Il monte l'eau du sol jusqu'aux feuilles."),
    };
    Info { name, state, fact }
}

/// How the palette names a state: as `info`, except the trunk, which plants a whole tree.
pub fn palette_info(m: Mat) -> Info {
    match m {
        Mat::Trunk => Info {
            name: "Arbre",
            state: "plante",
            fact: "Un clic plante un arbre adulte (tronc, branches et feuillage) sur le sol sous la souris. Le pinceau plus gros le fait plus grand. Il boit l'eau du sol, brûle comme du bois, et les voitures passent à travers.",
        },
        _ => info(m),
    }
}

/// A matter as the palette shows it: one button, with its states (solid, powder, liquid, gas)
/// behind a little arrow. The button paints `states[default]` until another state is picked.
/// Heat and cold move a cell from one state to the next by themselves (see `thermal::phase`).
pub struct Matter {
    pub states: &'static [Mat],
    pub default: usize,
}

const fn one(m: &'static [Mat]) -> Matter {
    Matter { states: m, default: 0 }
}

pub const MATTERS: [Matter; 17] = [
    one(&[Mat::Sand]),
    Matter { states: &[Mat::Ice, Mat::Snow, Mat::Water, Mat::Steam], default: 2 },
    one(&[Mat::Dirt]),
    one(&[Mat::Seed]),
    // Wood: a log, or behind the arrow a whole tree (its trunk stands for it in the palette).
    Matter { states: &[Mat::Wood, Mat::Trunk], default: 0 },
    one(&[Mat::Stone, Mat::Lava]),
    one(&[Mat::Iron, Mat::Molten]),
    one(&[Mat::Steel, Mat::MoltenSteel]),
    one(&[Mat::Glass]),
    one(&[Mat::Ash]),
    one(&[Mat::Charcoal]),
    one(&[Mat::Rust]),
    one(&[Mat::Saltpeter]),
    one(&[Mat::Gunpowder]),
    one(&[Mat::Diamond]),
    one(&[Mat::Clay]),
    one(&[Mat::Terracotta]),
];

impl Matter {
    /// The matter's name is that of its usual state: water for ice, snow and steam.
    #[cfg(test)]
    pub fn name(&self) -> &'static str {
        info(self.states[self.default]).name
    }
}

pub struct Recipe {
    /// Left side, shown as coloured swatches.
    pub from: &'static [Mat],
    pub to: &'static [Mat],
    pub text: &'static str,
}

pub const RECIPES: &[Recipe] = &[
    Recipe { from: &[Mat::Clay, Mat::Fire], to: &[Mat::Terracotta], text: "L'argile cuite vers 550 °C devient de la terre cuite : un creuset qui tient la chaleur et ne brûle pas." },
    Recipe { from: &[Mat::Wood, Mat::Fire], to: &[Mat::Ash], text: "Le bois brûle à l'air : il reste de la cendre (des minéraux). Le carbone est parti en fumée." },
    Recipe { from: &[Mat::Wood, Mat::Dirt, Mat::Fire], to: &[Mat::Charcoal], text: "La meule du charbonnier : du bois couvert de terre et chauffé sans air devient du charbon de bois." },
    Recipe { from: &[Mat::Fire], to: &[Mat::Smoke], text: "Un feu enfermé change l'air en fumée, puis s'éteint : il lui faut de l'oxygène." },
    Recipe { from: &[Mat::Iron, Mat::Charcoal, Mat::Fire], to: &[Mat::Steel], text: "Fer + carbone + chaleur (400 °C) : de l'acier. Dans un creuset de terre cuite sous un feu de bois, avec du charbon de bois pris à la pelle." },
    Recipe { from: &[Mat::Steel, Mat::Water], to: &[Mat::Rust], text: "L'eau rouille le fer, l'acier un peu moins vite." },
    Recipe { from: &[Mat::Rust, Mat::Fire], to: &[Mat::Iron], text: "La rouille chauffée avec du carbone (charbon de bois, ou les flammes d'un feu de bois) redonne du fer : le carbone lui prend son oxygène." },
    Recipe { from: &[Mat::Water, Mat::Fire], to: &[Mat::Steam], text: "L'eau bout à 100 °C et devient vapeur." },
    Recipe { from: &[Mat::Steam], to: &[Mat::Cloud, Mat::Water], text: "La vapeur refroidit : nuage, puis pluie. C'est le cycle de l'eau." },
    Recipe { from: &[Mat::Water], to: &[Mat::Steam], text: "L'eau s'évapore même froide, plus vite au chaud, à l'air sec et au vent. La vapeur, plus légère que l'air, monte faire des nuages." },
    Recipe { from: &[Mat::Dirt, Mat::Trunk, Mat::Leaf], to: &[Mat::Steam], text: "Les arbres boivent l'eau du sol par leurs racines et la rejettent en vapeur par leurs feuilles : une forêt rend l'air humide." },
    Recipe { from: &[Mat::Charcoal, Mat::Sand, Mat::Water], to: &[Mat::Charcoal, Mat::Water, Mat::Sand], text: "Archimède : ce qui est plus léger que l'eau flotte (charbon de bois, feuilles mortes, glace, bois), ce qui est plus lourd coule (sable, pierre, fer)." },
    Recipe { from: &[Mat::Water], to: &[Mat::Ice], text: "La nuit, quand il gèle, l'eau devient de la glace." },
    Recipe { from: &[Mat::Cloud], to: &[Mat::Water, Mat::Snow, Mat::Hail], text: "Nuage tiède : pluie. Gel : neige. Grand froid : grêle. Un nuage réchauffé par le soleil ne pleut pas." },
    Recipe { from: &[Mat::Sand, Mat::Fire], to: &[Mat::Glass], text: "Du sable très chaud fond : c'est du verre." },
    Recipe { from: &[Mat::Glass], to: &[Mat::Shard, Mat::Sand], text: "Le verre brisé redevient du sable avec le temps." },
    Recipe { from: &[Mat::Stone, Mat::Fire], to: &[Mat::Lava], text: "La roche chauffée longtemps fond en lave." },
    Recipe { from: &[Mat::Lava, Mat::Stone], to: &[Mat::Lava], text: "La lave fraîche est très chaude : la chaleur gagne la roche voisine qui fond à son tour." },
    Recipe { from: &[Mat::Iron, Mat::Charcoal, Mat::Fire], to: &[Mat::Molten], text: "Le fer fond vers 900 °C : un feu de bois (650 °C) ne suffit pas, mais la lave fraîche ou des braises de charbon de bois bien enfermées, si. Il rougit, jaunit, puis coule. En refroidissant, il se fige." },
    Recipe { from: &[Mat::Lava, Mat::Water], to: &[Mat::Stone], text: "La lave arrosée se fige en pierre." },
    Recipe { from: &[Mat::Stone, Mat::Ice], to: &[Mat::Sand], text: "Le gel et le dégel effritent la roche en sable." },
    Recipe { from: &[Mat::Dirt, Mat::Seed, Mat::Water], to: &[Mat::Trunk, Mat::Leaf], text: "Les graines semées se mêlent à la terre et la rendent fertile. Terre fertile + eau + lumière : un arbre pousse." },
    Recipe { from: &[Mat::Dirt, Mat::Water], to: &[Mat::Grass], text: "La terre boit l'eau. Humide et au soleil, l'herbe y pousse toute seule et s'étend ; quand la terre sèche, elle jaunit puis disparaît." },
    Recipe { from: &[Mat::Dirt, Mat::Water], to: &[Mat::Steam], text: "La terre mouillée sèche : son eau repart en vapeur, plus vite au chaud, à l'air sec et au vent, et plus vite encore sous l'herbe, qui transpire." },
    Recipe { from: &[Mat::Wood, Mat::Dirt], to: &[Mat::Oil], text: "Du bois enterré très longtemps devient du pétrole. Il flotte sur l'eau et brûle très fort." },
    Recipe { from: &[Mat::DeadLeaf, Mat::Ash], to: &[Mat::Saltpeter, Mat::Dirt], text: "La nitrière : des feuilles mortes qui pourrissent contre de la cendre de bois laissent du salpêtre, des cristaux blancs. À garder à l'abri de la pluie : l'eau le dissout." },
    Recipe { from: &[Mat::Saltpeter, Mat::Charcoal], to: &[Mat::Gunpowder], text: "Salpêtre + charbon de bois, mélangés à froid et au sec (à la pelle) : de la poudre noire. La vraie contient aussi un peu de soufre, qui la rend plus vive." },
    Recipe { from: &[Mat::Gunpowder, Mat::Fire], to: &[Mat::Smoke, Mat::Fire], text: "La poudre explose : beaucoup de gaz, une onde de pression qui casse la roche et pousse tout. La nuit, elle lance un feu d'artifice : ses couleurs viennent de sels de métaux (cuivre bleu, strontium rouge, sodium jaune, baryum vert)." },
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
        Mat::Charcoal => (38, 36, 38),
        Mat::Saltpeter => (238, 240, 232),
        Mat::Rust => (156, 84, 42),
        Mat::Glass => (190, 226, 246),
        Mat::Shard => (196, 230, 246),
        Mat::Snow => (244, 247, 252),
        Mat::Hail => (210, 236, 252),
        Mat::Oil => (40, 34, 36),
        Mat::Molten | Mat::MoltenSteel => (255, 210, 110),
        Mat::Smoke => (96, 96, 104),
        Mat::Cloud => (246, 248, 253),
        Mat::Ember => (255, 120, 24),
        Mat::Brick => (184, 72, 56),
        Mat::Clay => (168, 124, 98),
        Mat::Terracotta => (196, 98, 60),
        Mat::Vault => (170, 86, 58),
        Mat::Diamond => (196, 236, 246),
        Mat::Dynamite => (196, 46, 38),
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
    fn each_state_belongs_to_one_matter_and_clouds_are_left_to_the_sky() {
        let mut seen = vec![];
        for m in &MATTERS {
            assert!(m.default < m.states.len());
            for s in m.states {
                assert!(!seen.contains(s), "{s:?} is in two matters");
                seen.push(*s);
            }
        }
        // Clouds form from vapour by themselves; the palette paints vapour.
        assert!(!seen.contains(&Mat::Cloud));
        let water = MATTERS.iter().find(|m| m.states.contains(&Mat::Water)).unwrap();
        assert_eq!(water.states, &[Mat::Ice, Mat::Snow, Mat::Water, Mat::Steam]);
        assert_eq!(water.name(), "Eau");
        // Grass grows by itself on moist earth: it is not painted.
        assert!(!seen.contains(&Mat::Grass));
        // A metal and its melt are one matter.
        assert!(MATTERS.iter().any(|m| m.states == [Mat::Iron, Mat::Molten]));
        // Fire is lit with the flamethrower: it is not painted.
        assert!(!seen.contains(&Mat::Fire));
    }

    #[test]
    fn recipes_only_use_real_materials() {
        for r in RECIPES {
            assert!(!r.from.is_empty() && !r.to.is_empty() && !r.text.is_empty());
        }
    }
}
