//! The character values on the retail item page.
use super::{InventoryArt, InventoryScreen, RetailLabel, absolute, format_gold, info};
use crate::content::characters::Attribute;
use crate::content::items::ItemCatalog;
use crate::game::characters::Load;
use crate::game::combat::attack::armour;
use crate::game::data::GameData;
#[cfg(test)]
use crate::game::inventory::Carried;
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
        BitmapText {
            font: art.font.clone(),
            at,
            width,
            justify: bevy::text::Justify::Center,
            ..default()
        },
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
    screen: Option<Single<&InventoryScreen>>,
    data: Option<Res<GameData>>,
    catalogs: Option<Res<Assets<ItemCatalog>>>,
    members: Query<CharacterView>,
    art: Option<Res<InventoryArt>>,
    mut values: Query<(&Value, &mut BitmapText)>,
    mut bars: Query<&mut PictureCrop, With<ExperienceBar>>,
    battle: Option<Single<&crate::game::combat::Battle>>,
) {
    let (Some(screen), Some(art)) = (screen, art) else { return };
    let Ok(view) = members.get(screen.member) else { return };
    let load = view.load();
    let CharacterViewItem { profile: character, vitals, carried, burden } = view;

    let catalog = data
        .as_ref()
        .and_then(|data| catalogs.as_ref().and_then(|catalogs| catalogs.get(&data.items)));
    let weight = burden.map(|burden| burden.weight);
    let armour = carried.zip(catalog).map(|(carried, catalog)| {
        let defending = battle.as_ref().is_some_and(|b| b.defends(screen.member));
        armour(character, carried, catalog, load, defending)
    });
    for (kind, mut text) in &mut values {
        let (value, palette) = match *kind {
            Value::Experience => (format_gold(character.experience), None),
            Value::Goal => (format_gold(character.experience_goal()), None),
            Value::HitPoints => (format!("{}/{}", vitals.hp, character.max_hp()), None),
            Value::Stamina => (format!("{}/{}", vitals.stamina, character.max_stamina()), None),
            Value::Load => weight.map_or((String::new(), None), |weight| {
                let palette = match load {
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
        bar.set_if_neq(PictureCrop::horizontal(0.0, (103.0 * progress.clamp(0.0, 1.0)).floor()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Character, Gender, Profession, Race, Vitals};

    #[test]
    fn removing_carried_items_clears_all_equipment_derived_labels() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<ItemCatalog>>()
            .insert_resource(InventoryArt::default())
            .add_systems(Update, refresh);
        let items = app.world_mut().resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog(vec![]));
        app.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let actor =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        app.world_mut().spawn(InventoryScreen { member: actor });
        let base = app.world_mut().spawn((Value::BaseArmour, BitmapText::default())).id();
        let average = app.world_mut().spawn((Value::AverageArmour, BitmapText::default())).id();
        app.update();
        assert!(!app.world().get::<BitmapText>(base).unwrap().value.is_empty());
        assert!(!app.world().get::<BitmapText>(average).unwrap().value.is_empty());
        app.update();
        app.world_mut().entity_mut(actor).remove::<Carried>();
        app.update();
        assert!(app.world().get::<BitmapText>(base).unwrap().value.is_empty());
        assert!(app.world().get::<BitmapText>(average).unwrap().value.is_empty());
    }
}
