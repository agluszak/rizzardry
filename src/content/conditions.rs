/// Retail's conditions, mildest first. Death is not one of them here: a
/// member is dead exactly when their health is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
pub enum Condition {
    Drained,
    Diseased,
    Irritated,
    Nauseated,
    Slowed,
    Afraid,
    Poisoned,
    Silenced,
    Hexed,
    Infatuated,
    Insane,
    Blind,
    Turncoat,
    Webbed,
    Asleep,
    Paralyzed,
    Unconscious,
    /// Taken from the party, which keeps their place: they do nothing and
    /// nothing reaches them until they are back.
    Missing,
}

/// Changes to an actor while a condition holds. Values come from
/// `Gameplay Mods.cpp`, `ApplyConditionModifiers`.
#[derive(Debug, serde::Deserialize)]
pub struct Modifier {
    pub condition: Condition,
    #[serde(default)]
    pub attributes: Vec<(super::characters::Attribute, i32)>,
    #[serde(default)]
    pub skills: i32,
    #[serde(default)]
    pub hit: i32,
    #[serde(default)]
    pub armour: i32,
    /// Whether it keeps a member out of their formation place (retail
    /// `out_of_formation`).
    #[serde(default)]
    pub unseats: bool,
    #[serde(default)]
    pub eased_by: Option<Easing>,
}

/// A trait that spares its bearer a condition's worst: they keep their
/// place, and get back so much of an attribute, scaled by their profession
/// level.
#[derive(Debug, serde::Deserialize)]
pub struct Easing {
    pub ability: super::characters::Trait,
    pub attribute: super::characters::Attribute,
    pub back: f32,
}

pub static MODIFIERS: std::sync::LazyLock<Vec<Modifier>> = std::sync::LazyLock::new(|| {
    ron::from_str(include_str!("../../assets/tables/conditions.ron"))
        .expect("conditions.ron is a table of condition modifiers")
});
