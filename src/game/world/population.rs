//! Putting a level's monsters and items in their places when it is built.

use std::collections::HashMap;

use super::navigation::{LevelNavigation, WalkerMotionConfig};
use crate::game::world::level::LevelOwned;
use bevy::prelude::*;

use super::identify::{GroupMember, MonsterGroup};
use crate::content::level::LevelDefinition;
use crate::content::monster::MonsterBody;
use crate::content::species::MonsterCatalog;
use crate::game::data::Catalogs;
use crate::game::rng::GameRng;
use crate::game::world::items::place_item;

/// A monster in the level: which one (its level-unique location id), what
/// it is and its body. Its health, stamina, conditions and spells are its
/// [`Vitals`](crate::game::characters::Vitals).
#[derive(Component, Clone, Debug)]
pub struct Monster {
    pub location: i32,
    pub species: u32,
    pub body: ScaledBody,
}

/// A monster to be made: what its species rolled for it (retail
/// `LoadMonsterInfo`, `CreateMonsterInfo`), where it will stand and which
/// group it will be in. It becomes an actor of its level when its group is
/// born or it is placed, and is then the actor, not this.
#[derive(Clone, Debug)]
pub struct Newborn {
    pub group: Option<i32>,
    pub monster: Monster,
    pub vitals: crate::game::characters::Vitals,
    /// The health and stamina it was born with.
    pub frame: crate::game::actors::Frame,
    /// What ails it as it is made: nothing, unless it stood somewhere before.
    pub conditions: crate::game::characters::Conditions,
    pub budget: SpellBudget,
    pub feet: Vec3,
    pub yaw: f32,
    /// The script it is placed with.
    pub script: Option<String>,
}

impl Newborn {
    /// What it is told to do from where it stands, if it was placed with a
    /// script.
    fn bidden(&self) -> Option<(super::bidden::Script, super::bidden::Home)> {
        let script = self.script.clone()?;
        Some((super::bidden::Script(script), super::bidden::Home(self.feet)))
    }

    /// A placed monster met for the first time (retail `LoadMonsterInfo`).
    pub fn placed(
        placed: &crate::content::level::MonsterPlacement,
        kind: &crate::content::species::Species,
        body: &MonsterBody,
        rng: &mut GameRng,
    ) -> Self {
        let rolled = Self::rolled(
            (placed.location, placed.species, Some(placed.group)),
            (placed.position, placed.yaw),
            (kind, body),
            rng,
        );
        Self { script: placed.script.clone(), ..rolled }
    }

    /// A monster come into being (retail `CreateMonsterInfo`): health and
    /// stamina rolled from its species, then its size. Retail draws
    /// Random(1000) for the size; its left-handed mirror draw, presentation
    /// only, is not taken from gameplay's stream.
    pub fn rolled(
        (location, species, group): (i32, u32, Option<i32>),
        (feet, yaw): (Vec3, f32),
        (kind, body): (&crate::content::species::Species, &MonsterBody),
        rng: &mut GameRng,
    ) -> Self {
        let max_hp = rng.roll(kind.hit_points).max(1);
        let max_stamina = rng.roll(kind.stamina);
        let body = ScaledBody::new(body, rng.below(1000));
        Newborn {
            monster: Monster { location, species, body },
            vitals: crate::game::characters::Vitals::full(max_hp, max_stamina),
            frame: crate::game::actors::Frame { health: max_hp, stamina: max_stamina },
            conditions: default(),
            budget: SpellBudget(kind.magic.budget.max(0) as u32),
            group,
            feet,
            yaw,
            script: None,
        }
    }
}

/// What a monster's casting stands on now (retail
/// `ChooseMonsterSpellPowerLevel`, `MonsterCastsSpell`): its species' budget
/// when met, drained away or raised by Might to Magic since, never below
/// nothing. It stands in for a caster's skill in the failure roll.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpellBudget(pub u32);

impl Monster {
    /// Its eyes, over its feet at `feet`.
    pub fn eye(&self, feet: Vec3) -> Vec3 {
        feet + Vec3::Y * self.body.eye_height
    }

    /// The gap between its body at `feet` and another's at `other_feet`,
    /// less both fight radii, never below nothing (retail
    /// `GetMonsterDistanceToMonster`).
    pub fn gap(&self, feet: Vec3, other: &Monster, other_feet: Vec3) -> f32 {
        (feet.distance(other_feet) - self.body.radius - other.body.radius).max(0.0)
    }
}

/// A monster's effective gameplay body at the size it was rolled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledBody {
    pub scale: f32,
    /// Reach is measured from this far out from its centre (its scaled
    /// fight radius), in metres.
    pub radius: f32,
    /// Its eyes above its feet (its scaled target height).
    pub eye_height: f32,
    /// How fast it walks, in metres a second; 0 for those that do not
    /// (retail's navigator scales speed with size, `Navigator::SetScale`).
    pub speed: f32,
}

impl ScaledBody {
    /// The body of `definition` at retail's size roll (below 1000).
    pub fn new(definition: &MonsterBody, size_roll: u32) -> Self {
        let scale = definition.scale_for(size_roll);
        ScaledBody {
            scale,
            radius: definition.fight_radius * scale,
            eye_height: definition.target_height * scale,
            speed: definition.movement_rate * scale,
        }
    }
}

/// The body definitions a level's monsters need, by species: held by its
/// entry while it loads, then by the played level (for monsters spawned
/// later) until it is left. Visual model construction is not a gameplay
/// prerequisite and is requested separately by presentation.
#[derive(Component, Debug, Default)]
pub struct MonsterBodies(pub HashMap<u32, Handle<MonsterBody>>);

impl MonsterBodies {
    /// A species' body definition, once loaded.
    pub fn get<'a>(
        &self,
        species: u32,
        assets: &'a Assets<MonsterBody>,
    ) -> Option<&'a MonsterBody> {
        assets.get(self.0.get(&species)?)
    }

    /// The bodies of every species a level may bring into play, loading any
    /// not yet requested, since a group called up in play must have its
    /// body ready: those it places, its random encounters and their
    /// companions, the elementals the party may summon, and whatever any of
    /// these can summon, in turn.
    pub fn for_level(
        geometry: &LevelDefinition,
        species: &MonsterCatalog,
        assets: &AssetServer,
    ) -> Self {
        let encountered: Vec<u32> = geometry
            .encounters
            .generators
            .iter()
            .flat_map(|generator| generator.table.iter().flat_map(|table| &table.entries))
            .map(|entry| entry.species)
            .collect();
        let companions: Vec<u32> = encountered
            .iter()
            .map(|&id| &species[id])
            .flat_map(|kind| kind.companions.iter().flatten().map(|&(id, _)| id))
            .collect();
        let mut spawnable: Vec<u32> =
            geometry.monsters.iter().map(|placed| placed.species).collect();
        spawnable.extend(encountered);
        spawnable.extend(companions);
        spawnable.extend(crate::game::magic::workings::ELEMENTALS);
        let mut next = 0;
        while let Some(&id) = spawnable.get(next) {
            if let Some(called) = species
                .kind(id)
                .and_then(|kind| crate::game::hits::specials::special(kind.special.kind?).summons)
                && !spawnable.contains(&called)
            {
                spawnable.push(called);
            }
            next += 1;
        }
        Self(
            spawnable
                .into_iter()
                .filter_map(|id| {
                    let kind = species.kind(id)?;
                    Some((id, assets.load(crate::content::paths::monster(&kind.model))))
                })
                .collect(),
        )
    }
}

/// The name an actor goes by in the world's inspector.
fn actor_name(species: &MonsterCatalog, (id, location): (u32, i32)) -> String {
    format!("{} #{location}", species[id].name)
}

/// A monster of a species magic never takes as a target or victim (retail
/// `untargetable`).
#[derive(Component, Debug)]
pub struct Untargetable;

/// How well a monster's kind hides outside a fight: the rating the Monster
/// Editor gives it (retail record 0x248). The party makes it out less far for
/// each point (`docs/rules/monsters.md` section 2).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camouflage(pub u8);

/// What a monster's death is worth in experience to a fight: its species'.
#[derive(Component, Clone, Copy, Debug)]
pub struct Worth(pub u32);

/// Killing it is something for its killer to speak of.
#[derive(Component, Clone, Copy, Debug)]
pub struct Notable;

/// How a body carries itself in a fight when it decides for itself: its
/// species' temper.
#[derive(Component, Clone, Copy, Debug)]
pub struct Temper(pub crate::content::species::Temper);

/// The special attack a body may make at its turn: its species'.
#[derive(Component, Clone, Copy, Debug)]
pub struct Special(pub crate::content::species::SpecialAttack);

/// What a body may cast at its turn when it decides for itself: the chance
/// it does, its ten spell slots, and whether a silence leaves it its
/// alchemy (retail `IsSpellBlockedForMonster`).
#[derive(Component, Clone, Copy, Debug)]
pub struct Spellcasting {
    pub chance: u8,
    pub spells: [Option<crate::content::spells::SpellId>; 10],
    pub silent_alchemy: bool,
}

impl Spellcasting {
    /// A species' casting.
    pub fn of(kind: &crate::content::species::Species) -> Self {
        use crate::content::species::MonsterKind;
        Self {
            chance: kind.magic.chance,
            spells: kind.magic.spells,
            silent_alchemy: matches!(
                kind.kind,
                MonsterKind::Ranger | MonsterKind::Alchemist | MonsterKind::Ninja
            ),
        }
    }
}

/// What a species makes one of its own of: its abilities, level, kind and
/// what that lets it do, its natural guard, resistances and mending, and
/// what it strikes with. Written once, as the monster is made.
pub(crate) fn made_of(kind: &crate::content::species::Species) -> impl Bundle {
    use crate::content::characters::{Attribute, Resist, Trait};
    use crate::content::species::{MonsterKind, Reach};
    use crate::game::actors::{Abilities, Arm, Kind, Level, NaturalArms, Nature, Traits};
    use crate::game::inventory::WeaponSlot;
    let abilities = Abilities(enum_map::EnumMap::from_fn(|attribute| {
        u32::from(match attribute {
            Attribute::Strength => kind.attributes.strength,
            Attribute::Intelligence => kind.attributes.intelligence,
            Attribute::Dexterity => kind.attributes.dexterity,
            Attribute::Speed => kind.attributes.speed,
            Attribute::Senses => kind.attributes.senses,
            Attribute::Piety | Attribute::Vitality => 1,
        })
    }));
    // What its kind lets it do is the trait a character has it by.
    let mut traits = enumset::EnumSet::new();
    if kind.kind == MonsterKind::Monk {
        traits |= Trait::EffectiveWhileBlind;
    }
    let nature = Nature {
        // Armour is 10 less the record's value, so that higher is always
        // better guarded.
        armour: 10 - i32::from(kind.evasion),
        armour_at: std::array::from_fn(|place| 10 - i32::from(kind.armour_by_location[place])),
        armour_by_mode: enum_map::EnumMap::from_fn(|mode| i32::from(kind.armour_by_mode[mode])),
        hit_locations: Some(kind.hit_locations),
        exposed: 2 * i32::from(kind.vulnerable_from_behind),
        fatigue_guards: true,
        reduction: i32::from(kind.damage_reduction),
        resistance: enum_map::EnumMap::from_fn(|realm| {
            Some(Resist::Flat(i32::from(kind.resistances[realm])))
        }),
        initiative: i32::from(kind.initiative),
        mending: (i32::from(kind.health_regeneration), i32::from(kind.stamina_regeneration)),
        death_immune: kind.death_immune,
        carry_factor: 0,
        keen_sight: if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 },
        stride: Some(
            kind.combat_move_range * crate::game::combat::monster::close::METRES_PER_MOVE_RANGE,
        ),
    };
    let arms = kind
        .attacks
        .iter()
        .map(|swung| Arm {
            slot: WeaponSlot::Primary,
            weapon: None,
            reach: swung.reach,
            modes: swung.modes,
            dice: swung.damage,
            punch: swung.damage,
            kick: swung.damage,
            rating: i32::from(swung.score),
            percent: 0,
            effects: swung.effects,
            slays: Vec::new(),
            attacks: u32::from(kind.attacks_per_round),
            initiative: 0,
            swings: u32::from(kind.swings_per_round.max(1)),
            extra_swings: 0,
            ranged: false,
            loaded: None,
            sure_throw: false,
            critical: None,
            practises: &[],
            weight: None,
            tires: false,
            modern: swung.tires_little(),
            fatigue_pierces: swung.tires_little(),
            sword: false,
            ambushes: kind.kind == MonsterKind::Rogue && swung.reach <= Reach::Touch,
            word: Some((swung.word, swung.named())),
            lands_on_party: Some(kind.hits_on_party),
            sound_class: swung.sound_class,
        })
        .collect();
    (
        abilities,
        Level(u32::from(kind.level)),
        Traits(traits),
        Kind(kind.kind),
        nature,
        NaturalArms(arms),
    )
}

/// Make a monster an actor of its level: standing at its feet and facing
/// its heading, in its group, and a walker if it walks.
pub(crate) fn embody(
    entity: &mut EntityCommands,
    root: Entity,
    (feet, yaw, body): (Vec3, f32, ScaledBody),
    (group, kind): (Option<Entity>, &crate::content::species::Species),
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
) {
    let ScaledBody { radius, speed, .. } = body;
    entity.insert((
        Transform::from_translation(feet + Vec3::Y * super::navigation::float_height(speed))
            .with_rotation(Quat::from_rotation_y(yaw)),
        LevelOwned(root),
    ));
    if let Some(group) = group {
        entity.insert(GroupMember(group));
    }
    let allegiance = super::hostility::Allegiance::of(kind);
    entity.insert(made_of(kind));
    entity.insert((
        allegiance,
        Worth(kind.experience),
        crate::game::combat::effort::Labours,
        Camouflage(kind.camouflage),
        Temper(kind.temper),
        Special(kind.special),
        Spellcasting::of(kind),
    ));
    if kind.notable {
        entity.insert(Notable);
    }
    if kind.untargetable {
        entity.insert(Untargetable);
    }
    if speed > 0.0 {
        super::navigation::insert_walker(entity, (feet, yaw), (radius, speed), navigation, configs);
    }
}

/// Where a group about `origin` facing `yaw` may stand (retail
/// `FindScatterPositions`): in ranks going back from it, `spacing` apart,
/// three abreast (five for more than ten), each spot after the first
/// jittered by up to a fifth of a step. A spot counts where `ground`
/// settles it (none when the ground falls too far) and, when `clear`
/// allows it there and `reaches` finds a way to it from the first. Returns
/// at most `count` spots, fewer when the ground will not hold them.
pub fn scatter(
    (origin, yaw, spacing): (Vec3, f32, f32),
    count: usize,
    rng: &mut GameRng,
    ground: impl Fn(Vec3) -> Option<Vec3>,
    clear: impl Fn(Vec3) -> bool,
    reaches: impl Fn(Vec3, Vec3) -> bool,
) -> Vec<Vec3> {
    const COLUMNS: [f32; 5] = [0.0, 1.0, -1.0, 2.0, -2.0];
    let columns = if count > 10 { 5 } else { 3 };
    let rotation = Quat::from_rotation_y(yaw);
    let (back, side) = (rotation * Vec3::Z * spacing, rotation * Vec3::X * spacing);
    let mut source = origin;
    let mut spots: Vec<Vec3> = Vec::new();
    for rank in 0..10 {
        for &column in &COLUMNS[..columns] {
            if spots.len() >= count {
                return spots;
            }
            let (jitter_rank, jitter_column) = if spots.is_empty() {
                (0.0, 0.0)
            } else {
                let jitter = |rng: &mut GameRng| rng.below(1000) as f32 * 0.0004 - 0.2;
                let rank = jitter(rng);
                (rank, jitter(rng))
            };
            let candidate =
                source + back * (rank as f32 + jitter_rank) + side * (column + jitter_column);
            let Some(spot) = ground(candidate).filter(|&spot| clear(spot)) else { continue };
            if spots.is_empty() {
                source = spot;
                spots.push(spot);
            } else if reaches(spot, source) {
                spots.push(spot);
            }
        }
    }
    spots
}

/// A group come into the level (retail `CreateGroup`): its members as
/// rolled and placed, under the group id given, with the stance something
/// set on it. Returns the group and its members.
pub fn spawn_group(
    commands: &mut Commands,
    root: Entity,
    members: Vec<Newborn>,
    (authored, set): (i32, Option<super::hostility::SetStance>),
    species: &MonsterCatalog,
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
) -> (Entity, Vec<Entity>) {
    let group =
        commands.spawn((MonsterGroup { authored, set, ..default() }, LevelOwned(root))).id();
    let members = members
        .into_iter()
        .map(|kept| spawn_member(commands, root, kept, group, species, (navigation, configs)))
        .collect();
    (group, members)
}

/// A group member as an actor of its level: named, standing where it was
/// placed, in its group.
fn spawn_member(
    commands: &mut Commands,
    root: Entity,
    kept: Newborn,
    group: Entity,
    species: &MonsterCatalog,
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
) -> Entity {
    let name = actor_name(species, (kept.monster.species, kept.monster.location));
    let (body, kind) = (kept.monster.body, &species[kept.monster.species]);
    let bidden = kept.bidden();
    let mut entity = commands.spawn((
        Name::new(name),
        kept.monster,
        kept.vitals,
        kept.frame,
        kept.conditions,
        kept.budget,
    ));
    embody(
        &mut entity,
        root,
        (kept.feet, kept.yaw, body),
        (Some(group), kind),
        (navigation, configs),
    );
    if let Some(bidden) = bidden {
        entity.insert(bidden);
    }
    entity.id()
}

/// On a level root: its monsters still unborn, as rolled with the rest
/// (retail `plsUnbornMonsterList`). They are not in the world, so they take
/// no part in anything until their group is born.
#[derive(Component, Clone, Debug, Default)]
pub struct Unborn(pub Vec<Newborn>);

/// The next free monster location and group ids of a level: past every
/// one its monsters and groups hold, the unborn's included.
pub fn next_ids(
    locations: impl IntoIterator<Item = i32>,
    groups: impl IntoIterator<Item = i32>,
    unborn: &Unborn,
) -> (i32, i32) {
    let next = |ids: &mut dyn Iterator<Item = i32>| ids.max().unwrap_or(0) + 1;
    (
        next(&mut locations.into_iter().chain(unborn.0.iter().map(|k| k.monster.location))),
        next(&mut groups.into_iter().chain(unborn.0.iter().filter_map(|k| k.group))),
    )
}

/// A group of `count` of a species called up in play, nine at most, rolled
/// in order under the ids `(location, authored)`: the first takes
/// `location`, the next the one after, and so on.
pub fn roll_group(
    (location, authored): (i32, i32),
    (id, count): (u32, u32),
    (kind, body): (&crate::content::species::Species, &MonsterBody),
    rng: &mut GameRng,
) -> Vec<Newborn> {
    (0..count.min(9) as i32)
        .map(|i| {
            Newborn::rolled(
                (location + i, id, Some(authored)),
                (Vec3::ZERO, 0.0),
                (kind, body),
                rng,
            )
        })
        .collect()
}

/// A monster the story brings into the level the party is in (retail
/// `SpawnMonsters`): one of a species at a named spot, standing toward the
/// party as told. It is there once its body has loaded.
#[derive(Component, Clone, Debug)]
pub struct Coming {
    pub species: u32,
    pub spot: String,
    pub stance: super::hostility::Stance,
    /// How many of them.
    pub count: u32,
    /// The named spot it faces as it arrives.
    pub faces: Option<String>,
    /// The script it follows from then on.
    pub script: Option<String>,
    /// How far from the spot it comes.
    pub shift: Vec3,
}

/// Those the story is bringing in arrive, each a group of its own.
pub fn come(
    mut commands: Commands,
    coming: Query<(Entity, &Coming)>,
    level: Single<(Entity, &super::level::Level, &LevelNavigation, &mut MonsterBodies, &Unborn)>,
    (monsters, groups): (Query<&Monster>, Query<&MonsterGroup>),
    definitions: Res<Assets<crate::content::level::LevelDefinition>>,
    (bodies, server): (Res<Assets<MonsterBody>>, Res<AssetServer>),
    (content, clock): (If<Res<Catalogs>>, Res<crate::game::time::GameClock>),
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    mut rng: ResMut<GameRng>,
) {
    if coming.is_empty() {
        return;
    }
    let (root, played, navigation, mut handles, unborn) = level.into_inner();
    let Some(definition) = definitions.get(&played.definition) else { return };
    for (request, arrival) in &coming {
        let (Some(kind), Some(feet)) =
            (content.species.kind(arrival.species), definition.spot(&arrival.spot))
        else {
            commands.entity(request).despawn();
            continue;
        };
        handles
            .0
            .entry(arrival.species)
            .or_insert_with(|| server.load(crate::content::paths::monster(&kind.model)));
        let Some(shape) = handles.get(arrival.species, &bodies) else { continue };
        let ids = next_ids(
            monsters.iter().map(|monster| monster.location),
            groups.iter().map(|group| group.authored),
            unborn,
        );
        let facing = arrival.faces.as_deref().and_then(|spot| definition.spot(spot));
        let yaw = facing.map_or(0.0, |at| crate::game::party::body::yaw_toward(at - feet));
        let mut members =
            roll_group(ids, (arrival.species, arrival.count), (kind, shape), &mut rng);
        for member in &mut members {
            member.feet = feet + arrival.shift;
            member.yaw = yaw;
            member.script = arrival.script.clone();
        }
        let set = super::hostility::SetStance { stance: arrival.stance, at: clock.ms };
        spawn_group(
            &mut commands,
            root,
            members,
            (ids.1, Some(set)),
            &content.species,
            (navigation, &mut configs),
        );
        commands.entity(request).despawn();
    }
}

/// Birth an unborn group of the played level, by the level's id for it
/// (retail `GiveBirthToMonster`): its members come into the world where
/// they were placed. A group already born, or with no unborn members, is
/// left as it is.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Birth(pub i32);

/// Bring unborn groups into the world as they are born.
pub fn give_birth(
    mut commands: Commands,
    mut births: MessageReader<Birth>,
    mut levels: Query<(Entity, &mut Unborn, &LevelNavigation), With<super::level::Level>>,
    groups: Query<(Entity, &MonsterGroup, &LevelOwned)>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    content: If<Res<Catalogs>>,
) {
    let Ok((root, mut unborn, navigation)) = levels.single_mut() else {
        births.clear();
        return;
    };
    let species = &content.species;
    for &Birth(authored) in births.read() {
        let (born, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut unborn.0).into_iter().partition(|k| k.group == Some(authored));
        unborn.0 = waiting;
        if born.is_empty() {
            continue;
        }
        let group = groups
            .iter()
            .find(|(_, group, owner)| owner.0 == root && group.authored == authored)
            .map(|(entity, ..)| entity)
            .unwrap_or_else(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, LevelOwned(root))).id()
            });
        for kept in born {
            spawn_member(&mut commands, root, kept, group, species, (navigation, &mut configs));
        }
    }
}

/// Spawn the level's monsters and items under its root, as part of its first
/// activation: every placed monster, idling at its feet and facing its
/// heading (retail places them all at once on arrival), with health and
/// stamina rolled from its species in placement order, and the items as
/// authored.
pub fn populate(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelDefinition,
    catalogs: &Catalogs,
    (map, npcs): (&str, &mut crate::game::npcs::NpcPopulation),
    (handles, bodies): (&MonsterBodies, &Assets<MonsterBody>),
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
    rng: &mut GameRng,
) {
    let (species, items) = (&catalogs.species, &catalogs.items);
    // Every placed monster is rolled in placement order, the unborn too.
    let (unborn, monsters): (Vec<_>, Vec<_>) = geometry
        .monsters
        .iter()
        .filter(|placed| species.0[placed.species as usize].npc.is_none())
        .map(|placed| {
            let kind = &species.0[placed.species as usize];
            let body = handles.get(placed.species, bodies).unwrap();
            (placed.unborn, Newborn::placed(placed, kind, body, rng))
        })
        .partition(|&(unborn, _)| unborn);
    let unborn = unborn.into_iter().map(|(_, newborn)| newborn).collect();
    commands.entity(root).insert(Unborn(unborn));
    // The encounters' timers jitter after the placed monsters' rolls.
    let encounters = super::encounters::EncounterState::authored(&geometry.encounters, rng);
    commands.entity(root).insert(encounters);
    let mut groups: HashMap<i32, Entity> = HashMap::new();
    let mut actors = Vec::new();
    for (_, newborn) in monsters {
        let body = newborn.monster.body;
        let name = actor_name(species, (newborn.monster.species, newborn.monster.location));
        let id = newborn.monster.species;
        let bidden = newborn.bidden();
        let entity = commands
            .spawn((
                Name::new(name),
                newborn.monster,
                newborn.vitals,
                newborn.frame,
                newborn.conditions,
                newborn.budget,
            ))
            .id();
        actors.push((entity, newborn.feet, newborn.yaw, body, newborn.group, id, bidden));
    }
    let average = npcs.average_level();
    let mut placed_actors = std::collections::HashSet::new();
    // Unborn NPCs are not placed; NPCs are never born.
    for placed in geometry.monsters.iter().filter(|placed| !placed.unborn) {
        let kind = &species[placed.species];
        let Some(id) = kind.npc else { continue };
        let body = handles.get(placed.species, bodies).unwrap();
        let place = crate::game::npcs::Placing { placed, id, map, average };
        let Some(actor) = npcs.place(commands, place, catalogs, &mut placed_actors, rng, |rng| {
            Newborn::placed(placed, kind, body, rng)
        }) else {
            continue;
        };
        let name = actor_name(species, (placed.species, placed.location));
        commands.entity(actor.0).insert(Name::new(name));
        let (entity, feet, yaw, body, group) = actor;
        let bidden = placed
            .script
            .clone()
            .map(|script| (super::bidden::Script(script), super::bidden::Home(placed.position)));
        actors.push((entity, feet, yaw, body, group, placed.species, bidden));
    }
    let monster_count = actors.len();
    for (actor, feet, yaw, body, group, id, bidden) in actors {
        let group = group.map(|authored| {
            *groups.entry(authored).or_insert_with(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, LevelOwned(root))).id()
            })
        });
        embody(
            &mut commands.entity(actor),
            root,
            (feet, yaw, body),
            (group, &species[id]),
            (navigation, configs),
        );
        if let Some(bidden) = bidden {
            commands.entity(actor).insert(bidden);
        }
    }
    info!("placed {monster_count} monsters and {} items", geometry.items.len());
    for placement in &geometry.items {
        place_item(commands, root, placement, &items[placement.item]);
    }
}
