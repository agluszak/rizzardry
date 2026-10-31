//! The character values on the retail item page.
use super::{InventoryArt, InventoryScreen, RetailLabel, absolute, format_gold, info};
use crate::content::characters::Attribute;
use crate::game::characters::Load;
use crate::presentation::character_view::{CharacterView, CharacterViewItem};
use crate::presentation::screen_art::{BitmapText, Picture, PictureCrop};
use bevy::prelude::*;
use enumset::EnumSet;

#[derive(Component)]
pub(super) enum Value {
    Experience,
    Goal,
    HitPoints,
    Stamina,
    Load,
    BaseArmour,
    AverageArmour,
    Attribute(Attribute),
}
#[derive(Component)]
pub(super) struct ExperienceBar;

fn label(
    commands: &mut Commands,
    parent: Entity,
    art: &InventoryArt,
    at: Vec2,
    width: f32,
    value: Value,
) {
    let label = crate::presentation::screen_art::spawn_label(
        commands,
        parent,
        BitmapText::centred(art.font.clone(), at, width),
    );
    commands.entity(label).insert(value);
}
fn caption(
    commands: &mut Commands,
    parent: Entity,
    art: &InventoryArt,
    id: usize,
    at: Vec2,
    width: f32,
    centred: bool,
) {
    let label = crate::presentation::screen_art::spawn_label(
        commands,
        parent,
        BitmapText {
            font: art.font.clone(),
            at,
            width,
            justify: if centred { bevy::text::Justify::Center } else { bevy::text::Justify::Left },
            ..default()
        },
    );
    commands.entity(label).insert(RetailLabel(id));
}

pub(super) fn build(commands: &mut Commands, panel: Entity, root: Entity, art: &InventoryArt) {
    commands.spawn((
        ExperienceBar,
        Picture {
            sheet: art.leveling_bar.clone(),
            frame: 0,
            at: Vec2::new(448.0, 38.0),
            bounds: None,
        },
        PictureCrop::horizontal(0.0, 0.0),
        ChildOf(panel),
    ));
    for (id, x, y, width, center) in [
        (0x91b, 345., 10., 206., true),
        (0x91c, 350., 24., 101., false),
        (0x91d, 350., 38., 101., false),
        (0x91e, 324., 58., 69., false),
        (0x91f, 324., 72., 69., false),
        (0x920, 324., 86., 69., false),
        (0x921, 324., 114., 117., true),
        (0x922, 324., 128., 69., false),
        (0x923, 324., 142., 69., false),
    ] {
        caption(commands, panel, art, id, Vec2::new(x, y), width, center);
    }
    for (value, x, y, width) in [
        (Value::Experience, 448., 24., 103.),
        (Value::Goal, 448., 38., 103.),
        (Value::HitPoints, 397., 58., 44.),
        (Value::Stamina, 397., 72., 44.),
        (Value::Load, 397., 86., 44.),
        (Value::BaseArmour, 397., 128., 44.),
        (Value::AverageArmour, 397., 142., 44.),
    ] {
        label(commands, panel, art, Vec2::new(x, y), width, value);
    }
    for (i, attribute) in EnumSet::<Attribute>::all().iter().enumerate() {
        let y = 58.0 + 14.0 * i as f32;
        let name = [0x924, 0x925, 0x926, 0x927, 0x928, 0x92a, 0x929][i];
        caption(commands, panel, art, name, Vec2::new(450.0, y), 76.0, false);
        label(commands, panel, art, Vec2::new(530.0, y), 20.0, Value::Attribute(attribute));
        commands
            .spawn((
                info::InfoTarget { subject: info::Subject::Attribute(attribute), root },
                absolute(Vec2::new(450.0, y), Vec2::new(77.0, 13.0)),
                Pickable::default(),
                ChildOf(panel),
            ))
            .observe(info::open_info);
    }
    for (index, y) in [(0, 58.0), (1, 72.0), (3, 128.0), (3, 142.0), (4, 86.0)] {
        commands
            .spawn((
                info::InfoTarget { subject: info::Subject::Secondary(index), root },
                absolute(Vec2::new(324.0, y), Vec2::new(117.0, 13.0)),
                Pickable::default(),
                ChildOf(panel),
            ))
            .observe(info::open_info);
    }
    for (i, frame) in [22, 18, 14, 21, 16, 24].into_iter().enumerate() {
        let at = Vec2::new(557.0, 13.0 + 24.0 * i as f32);
        commands.spawn((
            Picture { sheet: art.realms[i].clone(), frame, at, bounds: None },
            ChildOf(panel),
        ));
    }
}

pub(super) fn refresh(
    screen: Single<&InventoryScreen>,
    members: Query<CharacterView>,
    content: If<Res<crate::game::data::Catalogs>>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party_effects: Option<Single<&crate::game::magic::enchantments::PartyEffects>>,
    art: Res<InventoryArt>,
    mut values: Query<(&Value, &mut BitmapText)>,
    mut bars: Query<&mut PictureCrop, With<ExperienceBar>>,
) {
    let view = members.get(screen.member).unwrap();
    let load = view.load();
    let armour = view.armour(
        screen.member,
        &content.items,
        battle.as_deref().copied(),
        party_effects.as_deref().copied(),
    );
    let CharacterViewItem { profile: character, vitals, burden, .. } = view;

    let weight = burden.weight;
    let stats = character.stats(&vitals.enchantments);
    for (kind, mut text) in &mut values {
        let (value, palette) = match *kind {
            Value::Experience => (format_gold(character.experience), None),
            Value::Goal => (format_gold(character.experience_goal()), None),
            Value::HitPoints => (format!("{}/{}", vitals.hp, stats.max_hp()), None),
            Value::Stamina => (format!("{}/{}", vitals.stamina, stats.max_stamina()), None),
            Value::Load => {
                let palette = match load {
                    Load::None => None,
                    Load::Light => Some(art.blue.clone()),
                    Load::Medium => Some(art.green.clone()),
                    Load::Heavy => Some(art.yellow.clone()),
                    Load::Extreme => Some(art.red.clone()),
                };
                (format!("{}/{}", weight / 10, stats.capacity() / 10), palette)
            }
            Value::BaseArmour => (armour.total.to_string(), None),
            Value::AverageArmour => (armour.average().to_string(), None),
            Value::Attribute(attribute) => (stats.attribute(attribute).to_string(), None),
        };
        if text.value != value {
            text.value = value;
        }
        if text.palette != palette {
            text.palette = palette;
        }
    }
    let progress = character.experience.saturating_sub(character.experience_previous_goal) as f32
        / character.experience_goal().saturating_sub(character.experience_previous_goal).max(1)
            as f32;
    for mut bar in &mut bars {
        bar.set_if_neq(PictureCrop::horizontal(0.0, (103.0 * progress.clamp(0.0, 1.0)).floor()));
    }
}
