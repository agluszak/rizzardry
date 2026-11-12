//! What the story has to say when a device of a level is used (retail's
//! activation callbacks, `InitializeLevelMasterFunctions`): by the level and
//! the device's name, the first case whose reading holds sets off what
//! follows and says whether the device then does what it was made to.
use std::collections::HashMap;
use std::sync::LazyLock;

use bevy::prelude::*;
use serde::Deserialize;

use super::story::{Befalls, Consequence, Due, Reading, Readings, StoryFacts, StoryTold};
use crate::content::characters::BodyPart;
use crate::content::items::AttackMode;
use crate::content::items::{Dice, ItemId};
use crate::content::level::LevelDefinition;
use crate::content::levels::LevelInfo;
use crate::content::spells::{SpellId, Targeting};
use crate::game::actors::Armour;
use crate::game::actors::Laying;
use crate::game::characters::{Character, Vitals};
use crate::game::combat::attack::{MEMBER_LOCATIONS, locate_at, mode_penetration};
use crate::game::data::Catalogs;
use crate::game::factions::Factions;
use crate::game::hits::Damaged;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::items::{Items, Lose, Place, Put, make, roll_amount};
use crate::game::magic::landed::Magical;
use crate::game::magic::power;
use crate::game::party::damage_member;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::encounters::EncounterState;
use crate::game::world::level::{Level, LevelOwned, LevelReady, SetAside};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;
use crate::game::world::population::{Birth, MonsterBodies, Unborn};
use crate::game::world::props::PropMotion;
use crate::game::world::triggers::{self, Activating, Device, DeviceNames, TriggerEffect};
use avian3d::prelude::{ColliderAabb, Position};

/// One case of a device's gate.
#[derive(Debug, Deserialize)]
struct Case {
    #[serde(default)]
    when: Reading,
    #[serde(default)]
    then: Vec<Consequence>,
    /// Whether the device then does what it was made to.
    #[serde(default = "runs")]
    runs: bool,
}

fn runs() -> bool {
    true
}

/// What the story has to say in one level.
#[derive(Debug, Default, Deserialize)]
struct LevelRows {
    /// As the party enters: every case that holds, in order.
    #[serde(default)]
    enter: Vec<Case>,
    /// Its devices' gates, by name.
    #[serde(default)]
    devices: HashMap<String, Vec<Case>>,
}

/// Every level's rows, by the three letters of the level.
static GATES: LazyLock<HashMap<String, LevelRows>> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/devices.ron"))
        .expect("assets/tables/devices.ron is a table of gates")
});

/// The party enters a level (retail's level setups, run as a level loads):
/// every case of the level's that holds sets off what it names.
pub(super) fn enter(
    mut ready: MessageReader<LevelReady>,
    level: Single<&Level, Without<SetAside>>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    if ready.read().count() == 0 {
        return;
    }
    let rows = LevelInfo::for_path(&level.path).and_then(|info| GATES.get(info.location));
    for case in rows.into_iter().flat_map(|rows| &rows.enter) {
        if readings.answer(&case.when, None, &story, &factions) != 0 {
            commands.spawn(Due::now(case.then.clone()));
        }
    }
}

/// On a device the story has a say in: its cases, in order.
#[derive(Component, Debug)]
pub struct Gate(&'static [Case]);

/// The devices of a level the party enters that the story has a say in are
/// given their gates. A name several devices share is the last one's, as
/// the original finds it.
pub(super) fn gate_devices(
    devices: Query<(Entity, &Device, &LevelOwned), Added<Device>>,
    levels: Query<&Level>,
    definitions: Res<Assets<LevelDefinition>>,
    mut commands: Commands,
) {
    for (device, made, owner) in &devices {
        let Ok(level) = levels.get(owner.0) else { continue };
        let gates = LevelInfo::for_path(&level.path).and_then(|info| GATES.get(info.location));
        let (Some(gates), Some(definition)) = (gates, definitions.get(&level.definition)) else {
            continue;
        };
        let name = &definition.triggers[made.definition].name;
        let last = definition.triggers.iter().rposition(|t| t.name.eq_ignore_ascii_case(name));
        let cases = gates.devices.iter().find(|(gated, _)| gated.eq_ignore_ascii_case(name));
        if let (Some((_, cases)), true) = (cases, last == Some(made.definition)) {
            commands.entity(device).insert(Gate(cases));
        }
    }
}

/// A gated device is used: its first case that holds decides whether it
/// goes on. What the case sets off befalls once the device is done.
pub(super) fn ask(
    mut asking: On<Activating>,
    gates: Query<&Gate>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    let Ok(Gate(cases)) = gates.get(asking.device) else { return };
    let holds = |case: &&Case| readings.answer(&case.when, None, &story, &factions) != 0;
    if let Some(case) = cases.iter().find(holds) {
        asking.proceeds = case.runs;
        if !case.then.is_empty() {
            commands.spawn(Due::now(case.then.clone()));
        }
    }
}

/// What a device's gate does to the party and its level: a message of the
/// level shown, experience won, a generator of encounters switched, the
/// party hurt, a spell cast where it stands, what its hand holds taken or
/// put there.
pub(super) fn befalls_at_a_device(
    befalls: On<Befalls>,
    party: Single<(Entity, &PartyRoster)>,
    mut members: Query<(&mut Character, &Vitals)>,
    mut levels: Query<(&Level, &mut EncounterState, &Unborn, &mut MonsterBodies)>,
    (definitions, server): (Res<Assets<LevelDefinition>>, Res<AssetServer>),
    held: Items,
    (content, mut rng): (Res<Catalogs>, ResMut<GameRng>),
    (mut effects, mut told, mut births): (
        MessageWriter<TriggerEffect>,
        MessageWriter<StoryTold>,
        MessageWriter<Birth>,
    ),
    mut commands: Commands,
) {
    let (party, roster) = *party;
    match &befalls.0 {
        Consequence::Tells(string) => {
            told.write(StoryTold { string: *string });
        }
        Consequence::Message(id) => {
            if let Some((level, ..)) = levels.iter().next() {
                effects.write(TriggerEffect::Message { source: level.messages.clone(), id: *id });
            }
        }
        // Each living member wins as much (retail `AwardPartyExperience`).
        Consequence::Experience(amount) => {
            for member in roster.iter() {
                if let Ok((mut character, vitals)) = members.get_mut(member)
                    && !vitals.is_dead()
                {
                    character.award_experience(*amount);
                }
            }
        }
        // The generators that draw from one table draw from another from
        // now on (retail `SetEncounterTable`); what it holds is made ready.
        Consequence::Breeds { from, to } => {
            for (level, mut state, _, mut bodies) in &mut levels {
                let Some(definition) = definitions.get(&level.definition) else { continue };
                let encounters = &definition.encounters;
                let Some(other) =
                    encounters.tables.iter().position(|table| table.name.eq_ignore_ascii_case(to))
                else {
                    continue;
                };
                for entry in &encounters.tables[other].entries {
                    let kinds = std::iter::once(entry.species).chain(
                        content.species[entry.species].companions.iter().flatten().map(|c| c.0),
                    );
                    for species in kinds {
                        let model = &content.species[species].model;
                        bodies
                            .0
                            .entry(species)
                            .or_insert_with(|| server.load(crate::content::paths::monster(model)));
                    }
                }
                for (index, generator) in encounters.generators.iter().enumerate() {
                    let named = generator.table.as_ref().map(|table| table.name.as_str());
                    if named.is_some_and(|named| named.eq_ignore_ascii_case(from))
                        && let Some(generator) = state.generators.get_mut(index)
                    {
                        generator.table = Some(other);
                    }
                }
            }
        }
        Consequence::Generator(name, on) => {
            for (level, mut state, ..) in &mut levels {
                let Some(definition) = definitions.get(&level.definition) else { continue };
                let named = &definition.encounters.generators;
                let named = named.iter().position(|g| g.name.eq_ignore_ascii_case(name));
                if let Some(generator) = named.and_then(|index| state.generators.get_mut(index)) {
                    generator.enabled = *on;
                }
            }
        }
        // Every group of a species still to be born is (retail
        // `GiveBirthToMonster` on each).
        Consequence::Births(species) => {
            for (_, _, unborn, _) in &levels {
                let of_species = unborn.0.iter().filter(|one| one.monster.species == *species);
                let groups: std::collections::BTreeSet<i32> =
                    of_species.filter_map(|one| one.group).collect();
                births.write_batch(groups.into_iter().map(Birth));
            }
        }
        Consequence::Hurt(count, sides, base) => {
            let dice = Dice { count: *count, sides: *sides, base: *base };
            commands.queue(move |world: &mut World| {
                let _ = world.run_system_cached_with(triggers::damage_party, dice);
            });
        }
        // A spell with no caster (retail `PointCastSpell`): one aimed at a
        // single being lands on a living member at random, any other on
        // every living member.
        Consequence::CastHere(spell, power) => {
            let id = SpellId(*spell);
            let Some(spell) = content.spells.get(id) else { return };
            let mut living: Vec<Entity> = roster
                .iter()
                .filter(|&member| members.get(member).is_ok_and(|(_, vitals)| !vitals.is_dead()))
                .collect();
            let single =
                matches!(spell.targeting, Targeting::Caster | Targeting::Ally | Targeting::Enemy);
            if single && !living.is_empty() {
                living = vec![living[rng.below(living.len() as u32) as usize]];
            }
            let strike = Strike {
                rows: Rows::Spell(id),
                strength: power::strength(0, *power, 0, spell),
                blow: None,
                by: None,
            };
            commands.spawn((Deed::at(strike, living), Magical));
        }
        Consequence::LoseHeld => {
            if let Some(item) = held.held(party) {
                commands.trigger(Lose { item, amount: 1 });
            }
        }
        Consequence::HandItem(item) => {
            let definition = &content.items[ItemId(*item)];
            let amount = roll_amount(definition, &mut rng, true);
            let item = make(&mut commands, definition, amount, true);
            commands.trigger(Put { item, at: Place::Hand { party } });
        }
        _ => {}
    }
}

/// How far a trap's missile flies (retail's 15000 units).
const MISSILE_REACH: f32 = 30.0;

/// A trap's launchers fire (retail `FireTrapLauncher`): a missile from each
/// named spot, along the way it faces. One that the party stands in the way
/// of, with nothing solid between, strikes one living member at random
/// where a blow lands on a member; it gets through armour on a roll of 1 to
/// 100 no more than 50, the thrust's own, and five for each point the
/// armour there is under 10 (`ResolveSpellMissileHit`), unless a Missile
/// Shield turns it aside.
pub(super) fn shoots(
    befalls: On<Befalls>,
    level: Single<&Level, Without<SetAside>>,
    definitions: Res<Assets<LevelDefinition>>,
    party: Single<(&Position, &PartyRoster)>,
    mut members: Query<(&mut Vitals, &Armour)>,
    obstacles: WorldObstacles,
    mut rng: ResMut<GameRng>,
    mut laying: Laying,
    (mut effects, mut damage): (MessageWriter<TriggerEffect>, MessageWriter<Damaged>),
) {
    let Consequence::Shoots { from, dice } = &befalls.0 else { return };
    let Some(definition) = definitions.get(&level.definition) else { return };
    let (at, roster) = *party;
    let dice = Dice { count: dice.0, sides: dice.1, base: dice.2 };
    for launcher in from {
        let (Some(start), Some(aim)) = (definition.spot(launcher), definition.aim(launcher)) else {
            continue;
        };
        let path = "data/sound/combat/blow_gun_attack_01.wav".to_owned();
        effects.write(TriggerEffect::Sound { path, volume: 1.0, at: Some(start) });
        // The nearest the missile comes to the party's middle.
        let along = (at.0 - start).dot(*aim).clamp(0.0, MISSILE_REACH);
        let nearest = start + *aim * along;
        let reach = crate::game::party::body::RADIUS;
        if nearest.with_y(0.0).distance(at.0.with_y(0.0)) > reach
            || !obstacles.clear(start, nearest)
        {
            continue;
        }
        let living: Vec<Entity> = roster
            .iter()
            .filter(|&member| members.get(member).is_ok_and(|(vitals, _)| !vitals.is_dead()))
            .collect();
        if living.is_empty() {
            continue;
        }
        let struck = living[rng.below(living.len() as u32) as usize];
        let Ok((mut vitals, armour)) = members.get_mut(struck) else { continue };
        let place = locate_at(rng.below(100), &MEMBER_LOCATIONS, BodyPart::Body as usize);
        let mode = AttackMode::Thrust;
        let there = armour.at[place.0] - armour.by_mode[mode];
        let aside = rng.below(100) < armour.missile_shield;
        let through = (rng.below(100) as i32) < 50 + mode_penetration(mode) + 5 * (10 - there);
        if aside || !through {
            continue;
        }
        let rolled = rng.roll(dice);
        if rolled > 0 {
            damage_member(struck, &mut vitals, rolled, &mut laying, &mut damage);
        }
    }
}

/// Two devices closing on each other along the level's first axis, and
/// what that does to a monster between them (retail
/// `MartensBluff2MonsterCrusher`).
#[derive(Component, Debug)]
pub struct Crushing {
    low: String,
    high: String,
    /// Of a hundred caught by both, so many are pushed clear.
    spared: u32,
    /// The species worth crushing, and what it is worth to each member.
    prize: (u32, u32),
    moved: bool,
}

pub(super) fn crush_begins(befalls: On<Befalls>, mut commands: Commands) {
    if let Consequence::Crushes { low, high, spared, prize } = &befalls.0 {
        let (low, high) = (low.clone(), high.clone());
        commands.spawn(Crushing { low, high, spared: *spared, prize: *prize, moved: false });
    }
}

/// While the plates move, a monster in their way is pushed by the one that
/// reaches it; one both reach dies, unless it is pushed clear.
pub(super) fn crush(
    mut crushing: Query<(Entity, &mut Crushing)>,
    names: Single<&DeviceNames, Without<SetAside>>,
    (parts, bounds, motions): (Query<&Children>, Query<&ColliderAabb>, Query<&PropMotion>),
    mut monsters: Query<(Entity, &Monster, &mut Position, &mut Vitals), Without<Character>>,
    (roster, mut members): (Single<&PartyRoster>, Query<(&mut Character, &Vitals)>),
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    // A device's bounds: those of the parts of its prop, together.
    let around = |device: Entity| {
        let parts = parts.get(device).ok()?;
        let boxes = parts.iter().filter_map(|part| bounds.get(part).ok());
        boxes.map(|b| (b.min, b.max)).reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)))
    };
    for (entity, mut crushing) in &mut crushing {
        let devices = (names.named(&crushing.low), names.named(&crushing.high));
        let (Some(low), Some(high)) = devices else {
            commands.entity(entity).despawn();
            continue;
        };
        let moving = motions.get(low).is_ok_and(|motion| motion.frame != motion.target);
        if !moving {
            if crushing.moved {
                commands.entity(entity).despawn();
            }
            crushing.moved = true;
            continue;
        }
        crushing.moved = true;
        let (Some(lower), Some(upper)) = (around(low), around(high)) else { continue };
        let (left, right) = (lower.1.x, upper.0.x);
        let (floor, roof) = (lower.0.min(upper.0), lower.1.max(upper.1));
        for (_body, monster, mut at, mut vitals) in &mut monsters {
            let within = (floor.y..=roof.y).contains(&at.y) && (floor.z..=roof.z).contains(&at.z);
            if vitals.is_dead() || !within || at.x < lower.0.x || at.x > upper.1.x {
                continue;
            }
            let radius = monster.body.radius;
            if left <= at.x - radius {
                if right < at.x + radius {
                    at.x = right - radius;
                }
            } else if at.x + radius <= right || rng.below(100) < crushing.spared {
                at.x = left + radius;
            } else {
                vitals.slay(None);
                if monster.species == crushing.prize.0 {
                    for member in roster.iter() {
                        if let Ok((mut character, vitals)) = members.get_mut(member)
                            && !vitals.is_dead()
                        {
                            character.award_experience(crushing.prize.1);
                        }
                    }
                }
            }
        }
    }
}

/// Something the story does once a named device of the level has stopped
/// moving (retail's watches on a prop's animation).
#[derive(Component, Debug)]
pub struct Stilled {
    device: String,
    rows: Vec<Consequence>,
    /// It has been seen moving, or waited a moment for it to start.
    moved: bool,
}

pub(super) fn still(
    mut waits: Query<(Entity, &mut Stilled)>,
    names: Single<&DeviceNames, Without<SetAside>>,
    motions: Query<&PropMotion>,
    mut commands: Commands,
) {
    for (wait, mut stilled) in &mut waits {
        let motion = names.named(&stilled.device).and_then(|device| motions.get(device).ok());
        if motion.is_some_and(|motion| motion.frame != motion.target) {
            stilled.moved = true;
        } else if stilled.moved {
            commands.spawn(Due::now(std::mem::take(&mut stilled.rows)));
            commands.entity(wait).despawn();
        } else {
            // What sets it moving befalls in this same tick.
            stilled.moved = true;
        }
    }
}

/// A watch on a device begins.
pub(super) fn watch(befalls: On<Befalls>, mut commands: Commands) {
    if let Consequence::WhenStill(device, rows) = &befalls.0 {
        commands.spawn(Stilled { device: device.clone(), rows: rows.clone(), moved: false });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spots and the devices some consequences name.
    fn named<'a>(rows: &'a [Consequence], spots: &mut Vec<&'a str>, devices: &mut Vec<&'a str>) {
        for row in rows {
            match row {
                Consequence::SpawnMonster { at, faces, .. } => {
                    spots.push(at);
                    spots.extend(faces.as_deref());
                }
                Consequence::SpawnItem(_, spot)
                | Consequence::LookAtSpot(spot)
                | Consequence::Moved(_, spot)
                | Consequence::GroupMoved(_, spot) => spots.push(spot),
                Consequence::Shoots { from, .. } => spots.extend(from.iter().map(String::as_str)),
                Consequence::Trigger(name)
                | Consequence::TriggerOnce(name)
                | Consequence::TriggerOff(name)
                | Consequence::TriggerOn(name)
                | Consequence::TriggerEnabled(name)
                | Consequence::Unlock(name) => devices.push(name),
                Consequence::If(_, rows)
                | Consequence::Later(_, rows)
                | Consequence::AfterTalk(rows)
                | Consequence::Every(_, _, rows) => named(rows, spots, devices),
                Consequence::Crushes { low, high, .. } => {
                    devices.extend([low.as_str(), high.as_str()])
                }
                Consequence::WhenStill(device, rows) => {
                    devices.push(device);
                    named(rows, spots, devices);
                }
                Consequence::OneOf(choices) => {
                    for rows in choices {
                        named(rows, spots, devices);
                    }
                }
                _ => {}
            }
        }
    }

    /// Every gate is on a device its level really has.
    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_gates_name_devices_of_their_levels() {
        let files = crate::retail::files::retail_files();
        for (location, gates) in GATES.iter() {
            let info = crate::content::levels::LEVELS
                .iter()
                .find(|level| level.location == location)
                .unwrap_or_else(|| panic!("no level {location}"));
            let data = files.read(&info.path()).unwrap().unwrap();
            let start = crate::retail::pvl::compiled_level_end(&data).unwrap();
            let mut bytes = crate::bytes::Bytes::new(&data[start..]);
            let sections = crate::retail::sections::read_sections(&mut bytes).unwrap();
            for name in gates.devices.keys() {
                assert!(sections.trigger(name).is_some(), "{location} has no device {name}");
            }
            // Every spot and device a case names is the level's too.
            let cases = gates.devices.values().flatten().chain(&gates.enter);
            let (mut spots, mut devices) = (Vec::new(), Vec::new());
            for case in cases {
                named(&case.then, &mut spots, &mut devices);
            }
            for spot in spots {
                // A device is a spot by its own name.
                let found =
                    sections.positions.iter().any(|(name, _)| name.eq_ignore_ascii_case(spot))
                        || sections.trigger(spot).is_some();
                assert!(found, "{location} has no spot {spot}");
            }
            for device in devices {
                assert!(sections.trigger(device).is_some(), "{location} has no device {device}");
            }
        }
    }
}
