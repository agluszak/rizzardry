//! The sounds of blows (retail `Combat Sound.cpp`): a member's weapon heard
//! swung, and every blow that gets through heard where it lands, by the
//! weapon's sound class and what is struck there is made of, as the
//! original's `HitSounds.txt` pairs them.

use bevy::prelude::*;

use crate::content::hit_sounds::HitSounds;
use crate::game::combat::CombatNews;
use crate::game::data::Catalogs;
use crate::game::inventory::EquipSlot;
use crate::game::items::Items;
use crate::game::world::population::Monster;

#[derive(Resource)]
pub struct CombatSounds(Handle<HitSounds>);

pub fn load_combat_sounds(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(CombatSounds(assets.load("retail://data/databases/hitsounds.txt")));
}

/// Where a blow on a member lands, by the place the fight names, and the
/// slot whose armour it strikes there (retail `PCItemInACSlot`).
const STRUCK_SLOTS: [EquipSlot; 5] =
    [EquipSlot::Head, EquipSlot::Torso, EquipSlot::Legs, EquipSlot::Hands, EquipSlot::Feet];

pub fn play_combat_sounds(
    mut news: MessageReader<CombatNews>,
    sounds: Option<Res<CombatSounds>>,
    tables: Res<Assets<HitSounds>>,
    (monsters, held): (Query<&Monster>, Items),
    catalogs: Option<Res<Catalogs>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let table = sounds.and_then(|sounds| tables.get(&sounds.0));
    let (Some(table), Some(catalogs)) = (table, catalogs) else {
        news.clear();
        return;
    };
    for news in news.read() {
        let sound = match *news {
            CombatNews::Swung { sound_class: Some(class) } => {
                table.swings.get(usize::from(class)).map(String::as_str)
            }
            CombatNews::Struck { victim, place, sound_class: Some(class) } => {
                // A monster is struck in its own stuff; a member in what is
                // worn there, or bare.
                let material = match monsters.get(victim) {
                    Ok(monster) => catalogs.species[monster.species].material,
                    Err(_) => STRUCK_SLOTS
                        .get(place)
                        .and_then(|&slot| held.worn(victim, slot))
                        .and_then(|item| held.id(item))
                        .map_or(0, |id| catalogs.items[id].material),
                };
                table.impact(class, material)
            }
            _ => None,
        };
        if let Some(name) = sound {
            let path = format!("retail://data/sound/combat/{}.wav", name.to_lowercase());
            commands.spawn((
                Name::new(format!("Sound {name}")),
                AudioPlayer::new(assets.load(path)),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
