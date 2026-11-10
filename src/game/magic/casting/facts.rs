//! Reading the world for the rules of a cast: what each takes is a small struct
//! of facts, built here from one view of an actor ([`BodyReadOnlyItem`])
//! whichever of member and monster it is, from where everyone stands, and from
//! what the party is doing. Nothing here decides anything and nothing here
//! changes anything.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{Cast, Recast, Source};
use crate::content::characters::{Realm, Skill};
use crate::content::items::UseType;
use crate::content::spells::{SpellDefinition, SpellId, Targeting, Working};
use crate::game::actors::{Place, Places};
use crate::game::camp::Camp;
use crate::game::characters::{Character, Condition, Load, Profession};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::freedom::Freedom;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::{BodyReadOnly, BodyReadOnlyItem};
use crate::game::hits::reach::Struck;
use crate::game::hits::spells::struck;
use crate::game::items::{Casts, Items, Place as Stowed};
use crate::game::magic::pointcasts::Elementals;
use crate::game::magic::portals::Portal;
use crate::game::magic::reach::{self, Aim, Marking, Target};
use crate::game::magic::roll::{self, Pace, Presented};
use crate::game::magic::{best_book, may, power, total_caster_level, traps};
use crate::game::magic::{cost, may::UseContext};
use crate::game::npcs::conversation::Conversation;
use crate::game::npcs::{Npc, NpcBodies};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::interaction::{AttemptOn, DeviceInteraction, Guarded, Work};
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Spellcasting};
use crate::game::world::triggers::LevelState;

/// Whether a body at `from` reaches one at `to` with a spell of `reach`:
/// within it, body to body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `CanMonsterSeeMonster`'s line).
pub fn reaches(from: &Place, to: &Place, reach: f32, obstacles: &WorldObstacles) -> bool {
    reach::reached(from.gap(to), reach, obstacles.clear(from.eye, to.eye))
}

/// The failure chance of a learned spell at `power`, as of a pace.
pub fn learned_chance(
    character: &Character,
    spell: &SpellDefinition,
    power: u32,
    pace: Pace,
    on_oneself: bool,
) -> u32 {
    let book = character.skill(best_book(character, spell));
    let realm = spell.realm.map_or(0, |realm| character.skill(realm.skill()));
    roll::learned(&roll::Learned {
        spell,
        power,
        book,
        realm,
        caster_level: total_caster_level(character, spell.books),
        pace,
        on_oneself,
    })
}

/// What a cast looks at besides the bodies: where everyone stands, what the
/// party is doing, what is at hand.
#[derive(SystemParam)]
pub struct Around<'w, 's> {
    pub places: Places<'w, 's>,
    pub obstacles: WorldObstacles<'w, 's>,
    pub held: Items<'w, 's>,
    memberships: Query<'w, 's, &'static GroupMember>,
    groups: Query<'w, 's, &'static GroupMembers>,
    freedom: Freedom<'w, 's>,
    camp: Option<Res<'w, Camp>>,
    pub states: Query<'w, 's, &'static LevelState, With<Level>>,
    attempts: Query<'w, 's, (&'static DeviceInteraction, &'static AttemptOn)>,
    devices: Query<'w, 's, Has<Guarded>>,
    conversation: Option<Single<'w, 's, &'static Conversation>>,
    speakers: Query<'w, 's, (&'static Npc, Option<&'static NpcBodies>)>,
    monsters: Query<'w, 's, &'static LevelOwned, With<Monster>>,
    levels: Query<'w, 's, (), With<Level>>,
    portals: Query<'w, 's, &'static Portal>,
    elementals: Query<'w, 's, &'static Elementals>,
    spellcasting: Query<'w, 's, &'static Spellcasting>,
}

/// An item a cast uses, and what is known of it.
#[derive(Clone, Copy)]
pub struct ItemUse<'d> {
    pub item: Entity,
    pub casts: &'d Casts,
    pub identified: bool,
    worn: bool,
    spent: bool,
}

impl ItemUse<'_> {
    /// Potions and food are usable by anyone who holds them.
    pub fn by_anyone(&self) -> bool {
        matches!(self.casts.by, UseType::Potion | UseType::Food)
    }

    /// An instrument or gadget is played at the player's reach.
    pub fn played(&self) -> bool {
        matches!(self.casts.by, UseType::Instrument | UseType::Gadget)
    }
}

/// What a cast comes from, read once. Where the rules differ by it (the
/// power it goes off at, how likely it fails, what pays for it) they ask
/// this, so nothing else has to tell the three apart.
#[derive(Clone, Copy)]
pub enum Means<'b> {
    /// A learned spell at the power asked for, paid in points of its realm.
    Learned { character: &'b Character, asked: u32 },
    /// An item's spell, paid with one use of the item.
    Item { character: &'b Character, item: ItemUse<'b> },
    /// A monster's own casting at the power it chose: its budget stands in
    /// for skill, its level sets the strength, and it pays nothing.
    Own { power: u32, budget: u32, level: u8 },
}

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payment {
    /// Spell points of the spell's realm.
    Points,
    /// One use of an item.
    Use(Entity),
    Nothing,
}

impl<'b> Means<'b> {
    /// The item it uses, if it is one.
    pub fn item(&self) -> Option<&ItemUse<'b>> {
        match self {
            Means::Item { item, .. } => Some(item),
            _ => None,
        }
    }

    /// What a cast is paid with, whatever comes of the roll.
    pub fn payment(&self) -> Payment {
        match self {
            Means::Learned { .. } => Payment::Points,
            Means::Item { item, .. } => Payment::Use(item.item),
            Means::Own { .. } => Payment::Nothing,
        }
    }

    /// The member casting, whose skills are practised; a monster has none.
    pub fn character(&self) -> Option<&'b Character> {
        match *self {
            Means::Learned { character, .. } | Means::Item { character, .. } => Some(character),
            Means::Own { .. } => None,
        }
    }
}

/// The fight, the party and the catalogues a cast is read against.
pub struct Scene<'a, 'c, 'q, 'p, 'w, 's> {
    pub bodies: &'a Query<'q, 'p, BodyReadOnly>,
    pub around: &'a Around<'w, 's>,
    pub roster: &'a PartyRoster,
    pub combatants: &'a [Entity],
    pub content: &'c Catalogs,
    pub definitions: &'a Definitions,
    pub difficulty: Difficulty,
    /// The caster's moment in the round; 0 outside a fight.
    pub moment: u32,
    pub fighting: bool,
}

/// Everyone a spell may take, as the aim finds them.
pub struct Stage {
    bodies: Vec<Target>,
    party: Vec<Entity>,
    groups: Vec<Vec<Entity>>,
    eye: Vec3,
    forward: Vec3,
}

impl Stage {
    /// The facts the aim rules take.
    pub fn aiming(&self, caster: Entity, targeting: Targeting, aim: Aim) -> reach::Aiming<'_> {
        reach::Aiming {
            targeting,
            caster,
            aim,
            eye: self.eye,
            forward: self.forward,
            bodies: &self.bodies,
            party: &self.party,
            groups: &self.groups,
        }
    }
}

impl<'a, 'c, 'q, 'p> Scene<'a, 'c, 'q, 'p, '_, '_> {
    fn target(&self, body: Entity) -> Option<BodyReadOnlyItem<'_, 'p>> {
        self.bodies.get(body).ok().filter(|body| !body.untargetable)
    }

    /// Where the party is, which decides which spells suit the moment: a fight,
    /// the lock or trap being worked on, a conversation (for a spell cast at the
    /// one talked with), a camp, or exploring.
    pub fn context(&self, spell: &SpellDefinition) -> UseContext {
        if self.fighting {
            return UseContext::Combat;
        }
        if let Some((attempt, _)) = self.around.attempts.iter().next() {
            return match attempt.work {
                Work::Disarming(_) => UseContext::Trap,
                _ => UseContext::Lock,
            };
        }
        if self.around.conversation.is_some() && spell.facts.working.is_some_and(Working::speaks) {
            return UseContext::Conversation;
        }
        if self.around.camp.is_some() { UseContext::Camp } else { UseContext::World }
    }

    pub fn moment(&self, context: UseContext) -> may::Moment {
        may::Moment {
            context,
            occupied: !self.around.freedom.unoccupied(),
            camped: self.around.camp.is_some(),
            spoken_to: self.around.conversation.as_ref().is_some_and(|c| c.ready()),
        }
    }

    pub fn pace(&self) -> Pace {
        Pace { difficulty: self.difficulty, moment: self.moment }
    }

    /// Whether the party is under water, which quenches Fire.
    pub fn underwater(&self) -> bool {
        self.around.states.single().is_ok_and(|state| state.camera_sway)
    }

    /// The item a source uses, if it is one and is still there: carried by the
    /// caster, or in the party's pool.
    pub fn item_use(&self, source: Source, caster: Entity) -> Option<ItemUse<'a>> {
        let Source::Item(item) = source else { return None };
        let held: &'a Items = &self.around.held;
        if held.bearer(item) != Some(caster) && !held.pooled(item) {
            return None;
        }
        Some(ItemUse {
            item,
            casts: held.casts(item)?,
            identified: held.known(item)?.identified,
            worn: held.slot(item).is_some_and(|slot| slot.is_active()),
            spent: held.charges(item) == Some(0),
        })
    }

    /// What a cast that may be made comes from. None when what it names is
    /// not there to cast from, which the rule of who may cast has refused
    /// already.
    pub fn means<'b>(&self, source: Source, body: &BodyReadOnlyItem<'b, '_>) -> Option<Means<'b>>
    where
        'a: 'b,
    {
        Some(match source {
            Source::Innate { power, budget, level, .. } => Means::Own { power, budget, level },
            Source::Spell { power, .. } => {
                Means::Learned { character: body.character?, asked: power }
            }
            Source::Item(_) => Means::Item {
                character: body.character?,
                item: self.item_use(source, body.entity)?,
            },
        })
    }

    /// The power a cast goes off at: a monster's is the one it chose; an
    /// item's is its own, but an instrument or gadget is played, well or
    /// badly; a learned spell's is the power asked for as far as the points
    /// reach, or worked out for the caster when left to them. None when an
    /// instrument is played too badly to sound at all.
    pub fn power(
        &self,
        rng: &mut GameRng,
        means: &Means,
        body: &BodyReadOnlyItem,
        (cast, spell): (&Cast, &SpellDefinition),
        boost: u32,
    ) -> Option<u32> {
        Some(match means {
            Means::Own { power, .. } => *power,
            Means::Item { character, item } if item.played() => {
                match power::played(rng, self.played_chances(character, item, spell)) {
                    0 => return None,
                    power => power,
                }
            }
            Means::Item { item, .. } => item.casts.power,
            Means::Learned { asked, .. } => {
                let affordable = self.affordable(body, spell);
                if *asked != power::AUTO {
                    return Some(power::chosen(*asked, affordable, spell.power));
                }
                let identify = (spell.facts.working == Some(Working::Identify))
                    .then(|| self.identify_difficulty(cast))
                    .flatten()
                    .map(|difficulty| (difficulty, boost));
                power::auto(&self.auto(body, spell, cast, affordable, identify))
            }
        })
    }

    /// What the roll to cast is made against: how likely the cast fails at
    /// this power and moment of the round, whether a failure passes without
    /// notice, and whether water quenches it (the party under water casts no
    /// Fire).
    pub fn rolling(
        &self,
        means: &Means,
        spell: &SpellDefinition,
        power: u32,
        on_oneself: bool,
    ) -> roll::Rolling {
        let (chance, quiet) = match means {
            Means::Own { budget, .. } => (roll::spell_failure(*budget, spell, power), false),
            Means::Item { character, item } => {
                self.item_chance(character, item, spell, power, self.pace())
            }
            Means::Learned { character, .. } => {
                (learned_chance(character, spell, power, self.pace(), on_oneself), false)
            }
        };
        roll::Rolling {
            chance,
            quiet,
            quenched: spell.realm == Some(Realm::Fire) && self.underwater(),
            can_backfire: spell.can_backfire(),
        }
    }

    /// The caster, as the rule of who may cast asks of them.
    pub fn caster(&self, body: &BodyReadOnlyItem) -> may::Caster {
        may::Caster {
            able: body.vitals.can_fight(),
            member: body.member,
            silenced: body.vitals.conditions.has(Condition::Silenced),
            keeps_alchemy: match body.character {
                Some(character) => character.skill(Skill::Alchemy) > 0,
                None => self
                    .around
                    .spellcasting
                    .get(body.entity)
                    .is_ok_and(|casting| casting.silent_alchemy),
            },
        }
    }

    /// The highest power of a spell the caster's points pay for now.
    pub fn affordable(&self, body: &BodyReadOnlyItem, spell: &SpellDefinition) -> u32 {
        let (Some(character), Some((learned, pool)), Some(realm)) =
            (body.character, body.spells, spell.realm)
        else {
            return 0;
        };
        let maximum = character.max_points(body.attributes, learned, &self.content.spells);
        power::affordable(pool.available(realm, &maximum), spell.cost)
    }

    /// What a cast is paid for from.
    pub fn source(
        &self,
        cast: &Cast,
        body: &BodyReadOnlyItem,
        spell: &SpellDefinition,
    ) -> may::Source {
        match cast.choice.source {
            Source::Spell { spell: id, power } => may::Source::Learned {
                learned: body.spells.is_some_and(|(learned, _)| learned.0.contains(&id)),
                asked: power,
                affordable: self.affordable(body, spell),
            },
            Source::Item(item) => {
                let used = self.item_use(cast.choice.source, body.entity);
                may::Source::Item {
                    pooled: self.around.held.pooled(item),
                    held: used.map(|used| self.held(&used, body)),
                }
            }
            Source::Innate { .. } => may::Source::Own,
        }
    }

    fn held(&self, item: &ItemUse, body: &BodyReadOnlyItem) -> may::Held {
        let casts = item.casts;
        may::Held {
            power: Some(casts.power),
            played: item.played(),
            voiced: casts.by == UseType::Instrument,
            activation: may::Activation {
                has_use: !matches!(casts.by, UseType::None | UseType::Spellbook),
                must_be_worn: casts.worn,
                worn: item.worn,
                usable_by_user: body.character.is_some_and(|c| casts.users.admit(c)),
                by_anyone: item.by_anyone(),
            },
            spent: item.spent,
        }
    }

    /// Whether the game resolves a spell: it has a row, or is a utility spell.
    pub fn resolved(&self, spell: &SpellDefinition) -> bool {
        self.definitions.spell(spell.id).is_some() || spell.facts.working.is_some()
    }

    /// What the rule of who may cast asks.
    pub fn asks<'d>(
        &self,
        cast: &Cast,
        body: &BodyReadOnlyItem,
        spell: &'d SpellDefinition,
    ) -> may::Asks<'d> {
        may::Asks {
            spell,
            caster: self.caster(body),
            source: self.source(cast, body, spell),
            moment: self.moment(self.context(spell)),
            resolved: self.resolved(spell),
        }
    }

    /// The party and the monsters of the fight, in its order, as an aim finds
    /// them from `caster`: how it stands toward each, whether each is reached
    /// within `reach`, and the groups of the monsters.
    pub fn stage(&self, caster: Entity, reach: f32) -> Stage {
        let from = self.around.places.of(caster);
        let mine = self.target(caster).map(|body| body.standing());
        let mut ids: Vec<Entity> =
            crate::game::combat::participants(self.roster, self.combatants).collect();
        let mut groups: Vec<Vec<Entity>> = Vec::new();
        let mut group_ids: Vec<Entity> = Vec::new();
        let mut group_of = |id: Entity, ids: &mut Vec<Entity>| -> Option<usize> {
            let group = self.around.memberships.get(id).ok()?.0;
            if let Some(known) = group_ids.iter().position(|&g| g == group) {
                return Some(known);
            }
            let members: Vec<Entity> = self.around.groups.get(group).ok()?.iter().collect();
            for &mate in &members {
                if !ids.contains(&mate) {
                    ids.push(mate);
                }
            }
            group_ids.push(group);
            groups.push(members);
            Some(group_ids.len() - 1)
        };
        let mut grouped = Vec::new();
        let mut at = 0;
        while at < ids.len() {
            grouped.push(group_of(ids[at], &mut ids));
            at += 1;
        }
        let bodies = ids
            .iter()
            .zip(grouped)
            .filter_map(|(&id, group)| {
                let body = self.target(id)?;
                let place = self.around.places.of(id);
                Some(Target {
                    id,
                    member: self.roster.contains(&id),
                    dead: body.vitals.is_dead(),
                    stance: body.stance(),
                    toward: mine
                        .map_or(Stance::Neutral, |mine| hostility::between(mine, body.standing())),
                    reached: reaches(&from, &place, reach, &self.around.obstacles),
                    gap: from.gap(&place),
                    feet: place.feet,
                    eye: place.eye,
                    group,
                    joined: self.combatants.iter().position(|&c| c == id).unwrap_or(usize::MAX),
                })
            })
            .collect();
        Stage {
            bodies,
            party: self.roster.iter().collect(),
            groups,
            eye: from.eye,
            forward: from.forward(),
        }
    }

    /// The bodies a blast centred on `centre` may take, as it finds them.
    pub fn struck(&self, origin: Vec3) -> Vec<Struck> {
        struck(
            self.bodies,
            &self.around.places,
            &self.around.obstacles,
            origin,
            (self.roster, self.combatants),
        )
    }

    /// The figure a cast's strength grows from and its boost: a monster's level
    /// with no boost; an item user's level, or their Bard or Gadgeteer levels
    /// for an instrument or a gadget, with no boost; a learned spell's total
    /// caster level, boosted by Power Cast.
    pub fn figure(&self, means: &Means, spell: &SpellDefinition) -> (u32, u32) {
        match means {
            Means::Own { level, .. } => (u32::from(*level), 0),
            Means::Item { character, item } => {
                let figure = match item.casts.by {
                    UseType::Instrument => character.profession_levels[Profession::Bard],
                    UseType::Gadget => character.profession_levels[Profession::Gadgeteer],
                    _ => character.level(),
                };
                (u32::from(figure), 0)
            }
            Means::Learned { character, .. } => {
                let figure = total_caster_level(character, spell.books).max(0) as u32;
                let cast_skill = character.skill(Skill::PowerCast);
                (figure, power::boost(cast_skill, character.skill_available(Skill::PowerCast)))
            }
        }
    }

    /// The failure chance of an item's spell at `power`, and whether the use is
    /// quiet, as of a pace.
    pub fn item_chance(
        &self,
        character: &Character,
        item: &ItemUse,
        spell: &SpellDefinition,
        power: u32,
        pace: Pace,
    ) -> (u32, bool) {
        let use_type = item.casts.by;
        let presented = match use_type {
            UseType::Thrown => Presented::Thrown,
            UseType::Charged => Presented::Charged,
            UseType::Scroll => Presented::Scroll,
            UseType::Instrument => Presented::Instrument,
            UseType::Gadget => Presented::Gadget,
            _ => Presented::Other,
        };
        let player = match use_type {
            UseType::Instrument => Profession::Bard,
            _ => Profession::Gadgeteer,
        };
        roll::item(&roll::Item {
            spell,
            power,
            presented,
            skill: use_type.skill().map(|skill| character.skill(skill)),
            player_levels: u32::from(character.profession_levels[player]),
            identified: item.identified,
            pace,
        })
    }

    /// The failure chance of an instrument or gadget at each power, as it asks
    /// when played: without the round's pace.
    pub fn played_chances(
        &self,
        character: &Character,
        item: &ItemUse,
        spell: &SpellDefinition,
    ) -> [u32; 7] {
        let unpaced = Pace { difficulty: Difficulty::Normal, moment: 0 };
        std::array::from_fn(|power| {
            let identified = ItemUse { identified: true, ..*item };
            self.item_chance(character, &identified, spell, power as u32 + 1, unpaced).0
        })
    }

    /// What an auto power weighs for the victim aimed at.
    pub fn auto<'b>(
        &'b self,
        body: &BodyReadOnlyItem,
        spell: &'b SpellDefinition,
        cast: &Cast,
        affordable: u32,
        identify: Option<(u8, u32)>,
    ) -> power::Auto<'b> {
        let victim = cast.choice.aim.body().and_then(|aimed| self.bodies.get(aimed).ok());
        let row = self.definitions.spell(spell.id).map(|id| self.definitions.get(id));
        let character = body.character;
        let failure = std::array::from_fn(|power| {
            character.map_or(0, |character| {
                learned_chance(character, spell, power as u32 + 1, self.pace(), false)
            })
        });
        power::Auto {
            affordable,
            victim: victim.map(|victim| power::Victim {
                member: victim.member,
                health: victim.vitals.health,
                stamina: victim.vitals.stamina,
                conditions: &victim.vitals.conditions,
            }),
            cost: spell.cost,
            dice: spell.dice,
            restores: row.and_then(|row| row.restores),
            outlasts: row.map_or(&[], |row| &row.outlasts),
            failure,
            identify,
        }
    }

    /// What tires the caster after a cast.
    pub fn tiring(
        &self,
        body: &BodyReadOnlyItem,
        item: Option<&ItemUse>,
        spell: &SpellDefinition,
        power: u32,
    ) -> cost::Tiring {
        let toll = match item.map(|item| item.casts.by) {
            None => cost::Toll::Spell { level: spell.level, power },
            Some(UseType::Thrown) => cost::Toll::Thrown,
            Some(UseType::Instrument | UseType::Gadget) => {
                cost::Toll::Played { level: spell.level }
            }
            Some(_) => {
                cost::Toll::Item { diseased: body.vitals.conditions.has(Condition::Diseased) }
            }
        };
        cost::Tiring {
            toll,
            load: body.member.then(|| body.burden.map_or(Load::None, |burden| burden.load)),
            slowed: body.vitals.conditions.has(Condition::Slowed),
            hasted: body.attacks.hasted > 0.0,
        }
    }

    /// What a cast trains if it affected someone.
    pub fn training(
        &self,
        character: &Character,
        item: Option<&ItemUse>,
        spell: &SpellDefinition,
        power: u32,
    ) -> cost::Training {
        match item {
            None => cost::Training::Learned {
                level: spell.level,
                power,
                book: best_book(character, spell),
                realm: spell.realm.map(Realm::skill),
                power_cast: character.skill_available(Skill::PowerCast),
            },
            Some(item) => {
                cost::Training::Item { level: spell.level, power, skill: item.casts.by.skill() }
            }
        }
    }

    /// The item Identify Item is aimed at, as the party has it.
    pub fn identified_of(&self, cast: &Cast) -> Option<reach::Held> {
        let Aim::Item(target) = cast.choice.aim else { return None };
        // Something the party has: in hand, in the pool, or with a member.
        let known = self
            .around
            .held
            .place(target)
            .filter(|place| match place {
                Stowed::Slot { member, .. } | Stowed::Pack { member, .. } => {
                    self.roster.contains(member)
                }
                _ => true,
            })
            .and(self.around.held.known(target))?;
        Some(reach::Held {
            party_has_it: true,
            itself: cast.choice.source == Source::Item(target),
            known: known.identified && known.spell && known.curse,
        })
    }

    /// How hard the item aimed at is to identify.
    pub fn identify_difficulty(&self, cast: &Cast) -> Option<u8> {
        let Aim::Item(target) = cast.choice.aim else { return None };
        self.around.held.obscurity(target)
    }

    /// The living body of the one the party is talking with, in the level
    /// played, as dialogues read it.
    fn speaker(&self) -> Option<Entity> {
        let conversation = self.around.conversation.as_ref()?;
        let (_, bodies) = self.around.speakers.get(conversation.actor).ok()?;
        bodies?.iter().find(|&body| {
            self.around.monsters.get(body).is_ok_and(|level| self.around.levels.contains(level.0))
                && self.bodies.get(body).is_ok_and(|body| !body.vitals.is_dead())
        })
    }

    /// What a utility spell is aimed at, as the party's surroundings offer it.
    pub fn marking(&self, working: Working, cast: &Cast, spell: &SpellDefinition) -> Marking {
        let eye = self.around.places.of(cast.caster).eye;
        let aim = cast.choice.aim;
        let attempt = match aim {
            Aim::Device(on) => {
                self.around.attempts.get(on).ok().map(|(attempt, &AttemptOn(device))| {
                    reach::Attempt {
                        guarded: self.around.devices.get(device).unwrap_or(false),
                        kind: match &attempt.work {
                            Work::Picking(_) => reach::Device::Lock,
                            Work::Disarming(trap) => reach::Device::Trap {
                                resolved: traps::validate(trap.trap, self.definitions).is_ok(),
                            },
                            Work::Looting => reach::Device::Container,
                        },
                    }
                })
            }
            _ => None,
        };
        Marking {
            working,
            aim,
            spot: match aim {
                Aim::Point(at) => Some((eye.distance(at), self.around.obstacles.clear(eye, at))),
                _ => None,
            },
            reach: spell.reach.metres(),
            eye_height: crate::content::EYE_HEIGHT,
            item: self.identified_of(cast),
            portal: self.around.portals.contains(cast.caster),
            speaker: self.speaker(),
            elemental: self.around.elementals.get(cast.caster).is_ok_and(|summons| {
                summons.iter().any(|e| self.bodies.get(e).is_ok_and(|b| !b.vitals.is_dead()))
            }),
            attempt,
        }
    }
}

impl Recast {
    /// Whether to cast again: a victim is not yet whole in what the spell
    /// restores, and the caster's stamina and points last.
    pub fn wanted(&self, bodies: &Query<BodyReadOnly>, content: &Catalogs) -> bool {
        let Source::Spell { spell, .. } = self.choice.source else { return false };
        let spell = &content.spells[spell];
        let victims: Vec<power::Mending> = self
            .victims
            .iter()
            .filter_map(|&victim| bodies.get(victim).ok())
            .map(|body| power::Mending {
                dead: body.vitals.is_dead(),
                pool: if self.watches_stamina { body.vitals.stamina } else { body.vitals.health },
            })
            .collect();
        power::unmended(&victims)
            && bodies.get(self.caster).is_ok_and(|caster| goes_again(&caster, spell, content))
    }
}

/// Whether an auto cast's caster can go again: stamina past the spell's
/// level and points for it at power 1.
pub fn goes_again(caster: &BodyReadOnlyItem, spell: &SpellDefinition, content: &Catalogs) -> bool {
    let (Some(character), Some((learned, pool)), Some(realm)) =
        (caster.character, caster.spells, spell.realm)
    else {
        return false;
    };
    let maximum = character.max_points(caster.attributes, learned, &content.spells);
    power::goes_again(
        caster.vitals.stamina.current,
        spell.level,
        pool.available(realm, &maximum),
        spell.cost,
    )
}

/// Whether the game resolves a spell it is asked about: it has a row, or is a
/// utility spell (what menus offer).
pub fn resolvable(spell: &SpellDefinition, definitions: &Definitions) -> bool {
    definitions.spell(spell.id).is_some() || spell.facts.working.is_some()
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    member: Entity,
    character: &Character,
    held: &Items,
    spells: &crate::content::spells::SpellCatalog,
    definitions: &Definitions,
) -> Vec<(Entity, SpellId)> {
    held.carried(member)
        .filter_map(|item| {
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            Some((item, usable_item(character, item, held, spells, definitions, worn)?))
        })
        .collect()
}

/// The spell an item casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_item(
    character: &Character,
    item: Entity,
    held: &Items,
    spells: &crate::content::spells::SpellCatalog,
    definitions: &Definitions,
    worn: bool,
) -> Option<SpellId> {
    let cast = held.casts(item)?;
    let activation = may::Activation {
        has_use: !matches!(cast.by, UseType::None | UseType::Spellbook),
        must_be_worn: cast.worn,
        worn,
        usable_by_user: cast.users.admit(character),
        by_anyone: matches!(cast.by, UseType::Potion | UseType::Food),
    };
    (may::activates(&activation)
        && held.charges(item) != Some(0)
        && resolvable(spells.get(cast.spell)?, definitions))
    .then_some(cast.spell)
}
