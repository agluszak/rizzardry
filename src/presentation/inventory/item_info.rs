//! The character values on the retail item page.
use super::{InventoryArt, InventoryScreen, RetailLabel, absolute, format_gold, info, review};
use crate::content::characters::Attribute;
use crate::content::items::ItemCatalog;
use crate::game::characters::{Character, Load, Vitals};
use crate::game::combat::attack::armour;
use crate::game::data::GameData;
use crate::game::inventory::{Carried, PartyInventory, carried_weight, item_weight, pool_shares};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::{BitmapText, Picture};
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
    commands.spawn((
        BitmapText {
            font: art.font.clone(),
            at,
            width,
            justify: bevy::text::Justify::Center,
            ..default()
        },
        Node { position_type: PositionType::Absolute, ..default() },
        value,
        Pickable::IGNORE,
        ChildOf(parent),
    ));
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
    commands.spawn((
        BitmapText {
            font: art.font.clone(),
            at,
            width,
            justify: if centred { bevy::text::Justify::Center } else { bevy::text::Justify::Left },
            ..default()
        },
        Node { position_type: PositionType::Absolute, ..default() },
        RetailLabel(id),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}

pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) {
    let panel = commands
        .spawn((
            review::Page::Items,
            absolute(Vec2::ZERO, Vec2::new(640.0, 165.0)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let clip = commands
        .spawn((
            ExperienceBar,
            Node {
                overflow: Overflow::clip_x(),
                ..absolute(Vec2::new(448.0, 38.0), Vec2::new(0.0, 12.0))
            },
            Pickable::IGNORE,
            ChildOf(panel),
        ))
        .id();
    commands.spawn((
        Picture { sheet: art.leveling_bar.clone(), frame: 0, at: Vec2::ZERO, bounds: None },
        Node { position_type: PositionType::Absolute, ..default() },
        Pickable::IGNORE,
        ChildOf(clip),
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
            Node { position_type: PositionType::Absolute, ..default() },
            Pickable::IGNORE,
            ChildOf(panel),
        ));
    }
}

pub(super) fn refresh(
    screen: Option<Single<Ref<InventoryScreen>>>,
    party: Option<Res<PartyRoster>>,
    inventory: Option<Res<PartyInventory>>,
    data: Option<Res<GameData>>,
    catalogs: Option<Res<Assets<ItemCatalog>>>,
    members: Query<(Ref<Character>, Ref<Vitals>, Option<Ref<Carried>>)>,
    art: Option<Res<InventoryArt>>,
    mut values: Query<(Ref<Value>, &mut BitmapText)>,
    mut bars: Query<&mut Node, With<ExperienceBar>>,
) {
    let (Some(screen), Some(art)) = (screen, art) else { return };
    let Ok((character, vitals, carried)) = members.get(screen.member) else { return };
    let party_changed = party.as_ref().is_some_and(|roster| {
        roster.is_changed()
            || roster.0.iter().any(|entity| {
                members.get(*entity).is_ok_and(|(profile, vitals, carried)| {
                    profile.is_changed()
                        || vitals.is_changed()
                        || carried.is_some_and(|carried| carried.is_changed())
                })
            })
    });
    if !screen.is_changed()
        && !character.is_changed()
        && !vitals.is_changed()
        && !carried.as_ref().is_some_and(|carried| carried.is_changed())
        && !party_changed
        && !inventory.as_ref().is_some_and(|inventory| inventory.is_changed())
        && !data.as_ref().is_some_and(|data| data.is_changed())
        && !catalogs.as_ref().is_some_and(|catalogs| catalogs.is_changed())
        && !values.iter().any(|(value, _)| value.is_added())
    {
        return;
    }

    let catalog = data
        .as_ref()
        .and_then(|data| catalogs.as_ref().and_then(|catalogs| catalogs.get(&data.items)));
    let weight = party.as_ref().zip(inventory.as_ref()).zip(catalog).and_then(
        |((roster, inventory), catalog)| {
            let member_data: Vec<_> = roster
                .0
                .iter()
                .map(|entity| {
                    members.get(*entity).ok().and_then(|(profile, vitals, carried)| {
                        if vitals.is_dead() {
                            None
                        } else {
                            Some((
                                carried
                                    .as_deref()
                                    .map_or(0, |carried| carried_weight(carried, catalog)),
                                profile.capacity(),
                            ))
                        }
                    })
                })
                .collect();
            let pool = inventory.pool.iter().map(|item| item_weight(item, catalog)).sum();
            let shares = pool_shares(pool, &member_data);
            let index = roster.0.get_index_of(&screen.member)?;
            let own = carried.as_deref().map_or(0, |carried| carried_weight(carried, catalog));
            Some(own + shares[index])
        },
    );
    let armour = carried
        .as_deref()
        .zip(catalog)
        .map(|(carried, catalog)| armour(&character, carried, catalog));
    for (kind, mut text) in &mut values {
        let (value, palette) = match *kind {
            Value::Experience => (format_gold(character.experience), None),
            Value::Goal => (format_gold(character.experience_goal()), None),
            Value::HitPoints => (format!("{}/{}", vitals.hp, character.max_hp()), None),
            Value::Stamina => (format!("{}/{}", vitals.stamina, character.max_stamina()), None),
            Value::Load => weight.map_or((String::new(), None), |weight| {
                let palette = match character.load(weight) {
                    Load::None => None,
                    Load::Light => Some(art.blue.clone()),
                    Load::Medium => Some(art.green.clone()),
                    Load::Heavy => Some(art.yellow.clone()),
                    Load::Extreme => Some(art.red.clone()),
                };
                (format!("{}/{}", weight / 10, character.capacity() / 10), palette)
            }),
            Value::BaseArmour => {
                (armour.as_ref().map_or(String::new(), |armour| armour.total.to_string()), None)
            }
            Value::AverageArmour => {
                (armour.as_ref().map_or(String::new(), |armour| armour.average().to_string()), None)
            }
            Value::Attribute(attribute) => (character.attribute(attribute).to_string(), None),
        };
        if text.value != value {
            text.value = value;
        }
        if text.palette != palette {
            text.palette = palette;
        }
    }
    // The previous experience goal is not stored yet for characters who have
    // changed profession. Level-one characters always start at zero.
    let progress = if character.level == 1 {
        character.experience as f32 / character.experience_goal().max(1) as f32
    } else {
        0.0
    };
    for mut bar in &mut bars {
        let width = px((103.0 * progress.clamp(0.0, 1.0)).floor());
        if bar.width != width {
            bar.width = width;
        }
    }
}
