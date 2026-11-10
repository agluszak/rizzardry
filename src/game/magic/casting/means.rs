//! What a cast comes from and what it looks at (`docs/rules/casting.md`):
//! the means of a cast ([`Means`]), read once from the caster and the item,
//! which the rules ask wherever they differ by it; and what stands around
//! the caster ([`Around`]).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{Cast, Recast, Source};
use crate::content::characters::Skill;
use crate::content::items::UseType;
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId, Working};
use crate::game::actors::{Place, Places};
use crate::game::camp::Camp;
use crate::game::characters::{Character, Profession};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::freedom::Freedom;
use crate::game::hits::effects::{BodyReadOnly, BodyReadOnlyItem};
use crate::game::items::{Casts, Items, Place as Stowed};
use crate::game::magic::may::Refusal;
use crate::game::magic::may::{self, UseContext};
use crate::game::magic::pointcasts::Elementals;
use crate::game::magic::portals::Portal;
use crate::game::magic::reach::{Aim, Marked};
use crate::game::magic::roll::{self, Pace};
use crate::game::magic::{power, total_caster_level, traps};
use crate::game::npcs::conversation::Conversation;
use crate::game::npcs::{Npc, NpcBodies};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::interaction::{AttemptOn, DeviceInteraction, Guarded, Work};
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Spellcasting};
use crate::game::world::triggers::LevelState;

/// Whether a body at `from` reaches one at `to` with a spell of `reach`:
/// within it, body to body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `CanMonsterSeeMonster`'s line).
pub fn reaches(from: &Place, to: &Place, reach: f32, obstacles: &WorldObstacles) -> bool {
    from.gap(to) <= reach && obstacles.clear(from.eye, to.eye)
}

/// What the party is doing as a cast is asked, which decides which spells
/// suit the moment.
#[derive(SystemParam)]
pub struct Circumstances<'w, 's> {
    freedom: Freedom<'w, 's>,
    camp: Option<Res<'w, Camp>>,
    attempts: Query<'w, 's, &'static DeviceInteraction>,
    conversation: Option<Single<'w, 's, &'static Conversation>>,
    states: Query<'w, 's, &'static LevelState, With<Level>>,
}

/// What a utility spell may be aimed at: an item the party has, a lock or
/// trap being worked on, the one being talked with, a spot in sight.
#[derive(SystemParam)]
pub struct Marks<'w, 's> {
    places: Places<'w, 's>,
    obstacles: WorldObstacles<'w, 's>,
    held: Items<'w, 's>,
    attempts: Query<'w, 's, (&'static DeviceInteraction, &'static AttemptOn)>,
    devices: Query<'w, 's, Has<Guarded>>,
    conversation: Option<Single<'w, 's, &'static Conversation>>,
    speakers: Query<'w, 's, (&'static Npc, Option<&'static NpcBodies>)>,
    monsters: Query<'w, 's, &'static LevelOwned, With<Monster>>,
    levels: Query<'w, 's, (), With<Level>>,
    portals: Query<'w, 's, &'static Portal>,
    elementals: Query<'w, 's, &'static Elementals>,
}

/// An item a cast uses, and what is known of it.
#[derive(Clone, Copy)]
pub struct ItemUse<'d> {
    pub item: Entity,
    pub casts: &'d Casts,
    pub identified: bool,
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

impl<'b> Means<'b> {
    /// What a cast comes from: a monster's own casting as it chose it, a
    /// member's learned spell, or the item they use. None for a monster asked
    /// for a member's means, or an item that is no longer there or casts
    /// nothing.
    pub fn of(source: Source, caster: &BodyReadOnlyItem<'b, '_>, held: &'b Items) -> Option<Self> {
        Some(match source {
            Source::Innate { power, budget, level, .. } => Means::Own { power, budget, level },
            Source::Spell { power, .. } => {
                Means::Learned { character: caster.character?, asked: power }
            }
            Source::Item(item) => {
                if held.bearer(item) != Some(caster.entity) && !held.pooled(item) {
                    return None;
                }
                let used = ItemUse {
                    item,
                    casts: held.casts(item)?,
                    identified: held.known(item)?.identified,
                };
                Means::Item { character: caster.character?, item: used }
            }
        })
    }

    /// How an item is used, if the cast comes from one.
    pub fn used(&self) -> Option<UseType> {
        self.item().map(|item| item.casts.by)
    }

    /// The figure a cast's strength grows from and its boost: a monster's
    /// level with no boost; an item user's level, or their Bard or Gadgeteer
    /// levels for an instrument or a gadget, with no boost; a learned spell's
    /// total caster level, boosted by Power Cast.
    pub fn figure(&self, spell: &SpellDefinition) -> (u32, u32) {
        match self {
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

    /// How likely the cast fails at `power` and this moment of the round, in
    /// percent, and whether a failure passes without notice: a monster's by
    /// its budget alone, an item's by the skill it is used with, a learned
    /// spell's by its caster's skill and level.
    pub fn failure(
        &self,
        spell: &SpellDefinition,
        power: u32,
        pace: Pace,
        on_oneself: bool,
    ) -> (u32, bool) {
        match self {
            Means::Own { budget, .. } => (roll::spell_failure(*budget, spell, power), false),
            Means::Item { character, item } => {
                roll::item(character, (item.casts, item.identified), spell, power, pace)
            }
            Means::Learned { character, .. } => {
                (roll::learned(character, spell, power, pace, on_oneself), false)
            }
        }
    }
}

impl Circumstances<'_, '_> {
    /// Where the party is, which decides which spells suit the moment: a fight,
    /// the lock or trap being worked on, a conversation (for a spell cast at the
    /// one talked with), a camp, or exploring; and what else it is busy with.
    pub fn moment(&self, spell: &SpellDefinition, fighting: bool) -> may::Moment {
        let context = if fighting {
            UseContext::Combat
        } else if let Some(attempt) = self.attempts.iter().next() {
            match attempt.work {
                Work::Disarming(_) => UseContext::Trap,
                _ => UseContext::Lock,
            }
        } else if self.conversation.is_some() && spell.facts.working.is_some_and(Working::speaks) {
            UseContext::Conversation
        } else if self.camp.is_some() {
            UseContext::Camp
        } else {
            UseContext::World
        };
        may::Moment {
            context,
            occupied: !self.freedom.unoccupied(),
            camped: self.camp.is_some(),
            spoken_to: self.conversation.as_ref().is_some_and(|c| c.ready()),
        }
    }

    /// Whether the party is under water, which quenches Fire.
    pub fn underwater(&self) -> bool {
        self.states.single().is_ok_and(|state| state.camera_sway)
    }
}

/// Whether a silence leaves a caster their Alchemy: a member with the skill,
/// a monster of a kind that keeps it.
pub fn keeps_alchemy(caster: &BodyReadOnlyItem, kinds: &Query<&Spellcasting>) -> bool {
    match caster.character {
        Some(character) => character.skill(Skill::Alchemy) > 0,
        None => kinds.get(caster.entity).is_ok_and(|casting| casting.silent_alchemy),
    }
}

impl Marks<'_, '_> {
    /// Whether a utility spell has a target, and which
    /// (`docs/rules/casting.md` section 8): Identify Item an item the party
    /// has, not the item casting it, not wholly known; Return to Portal a
    /// portal saved; Charm and Mindread the living one the party is talking
    /// with; Lure and Summon Elemental a spot within the spell's reach and
    /// an eye's height that the caster sees, an elemental of the caster's
    /// not standing; Knock Knock and Divine Trap the attempt under way, on a
    /// device still guarded, a trap the game resolves.
    pub fn mark(
        &self,
        working: Working,
        cast: &Cast,
        spell: &SpellDefinition,
        (bodies, roster): (&Query<BodyReadOnly>, &PartyRoster),
        content: &Catalogs,
    ) -> Result<Marked, Refusal> {
        let alive = |body| bodies.get(body).is_ok_and(|body| !body.vitals.is_dead());
        let caster = cast.caster;
        match (working, cast.choice.aim) {
            (Working::Identify, Aim::Item(item)) => {
                // Something the party has: in hand, in the pool, or with a
                // member.
                let ours = self.held.place(item).is_some_and(|place| match place {
                    Stowed::Slot { member, .. } | Stowed::Pack { member, .. } => {
                        roster.contains(&member)
                    }
                    _ => true,
                });
                let known = self
                    .held
                    .known(item)
                    .is_some_and(|known| known.identified && known.spell && known.curse);
                let itself = cast.choice.source == Source::Item(item);
                if ours && self.held.known(item).is_some() && !itself && !known {
                    Ok(Marked::Item(item))
                } else {
                    Err(Refusal::NoTarget)
                }
            }
            (Working::SetPortal, _) => Ok(Marked::Nothing),
            (Working::ReturnToPortal, _) if self.portals.contains(caster) => Ok(Marked::Nothing),
            (Working::Charm | Working::Mindread, Aim::Body(aimed)) => {
                // The living body of the one talked with, in the level played.
                let speaker = self.conversation.as_ref().and_then(|conversation| {
                    let (_, standing) = self.speakers.get(conversation.actor).ok()?;
                    standing?.iter().find(|&body| {
                        self.monsters.get(body).is_ok_and(|level| self.levels.contains(level.0))
                            && alive(body)
                    })
                });
                if speaker == Some(aimed) {
                    Ok(Marked::Body(aimed))
                } else {
                    Err(Refusal::NoTarget)
                }
            }
            (Working::Lure | Working::Summon, Aim::Point(at)) => {
                let eye = self.places.of(caster).eye;
                let seen = eye.distance(at) <= spell.reach.metres() + crate::content::EYE_HEIGHT
                    && self.obstacles.clear(eye, at);
                let elemental = working == Working::Summon
                    && self.elementals.get(caster).is_ok_and(|summons| summons.iter().any(alive));
                if seen && !elemental { Ok(Marked::Spot(at)) } else { Err(Refusal::NoTarget) }
            }
            (Working::Knock | Working::DivineTrap, Aim::Device(on)) => {
                let Ok((attempt, &AttemptOn(device))) = self.attempts.get(on) else {
                    return Err(Refusal::NoTarget);
                };
                if !self.devices.get(device).unwrap_or(false) {
                    return Err(Refusal::NoTarget);
                }
                match &attempt.work {
                    Work::Picking(_) => Ok(Marked::Attempt(on)),
                    Work::Disarming(trap) => traps::validate(trap.trap, content)
                        .map(|()| Marked::Attempt(on))
                        .map_err(|_| Refusal::Unsupported),
                    Work::Looting => Err(Refusal::NoTarget),
                }
            }
            _ => Err(Refusal::NoTarget),
        }
    }
}

/// The highest power of a spell the caster's points pay for now.
pub fn affordable(
    caster: &BodyReadOnlyItem,
    spell: &SpellDefinition,
    spells: &SpellCatalog,
) -> u32 {
    let (Some(character), Some((learned, pool)), Some(realm)) =
        (caster.character, caster.spells, spell.realm)
    else {
        return 0;
    };
    let maximum = character.max_points(caster.attributes, learned, spells);
    power::affordable(pool.available(realm, &maximum), spell.cost)
}

/// The power a cast goes off at: a monster's is the one it chose; an item's
/// is its own, but an instrument or gadget is played, as high as its chance
/// to fail (without the round's pace, and as if identified) allows; a
/// learned spell's is the power asked for as far as the points reach, or
/// worked out for the caster when left to them. None when an instrument is
/// played too badly to sound at all.
pub fn power_of(
    rng: &mut GameRng,
    means: &Means,
    caster: &BodyReadOnlyItem,
    (cast, spell): (&Cast, &SpellDefinition),
    (boost, pace): (u32, Pace),
    bodies: &Query<BodyReadOnly>,
    held: &Items,
    content: &Catalogs,
) -> Option<u32> {
    Some(match means {
        Means::Own { power, .. } => *power,
        Means::Item { character, item } if item.played() => {
            let unpaced = Pace { difficulty: Difficulty::Normal, moment: 0 };
            let chances = std::array::from_fn(|power| {
                roll::item(character, (item.casts, true), spell, power as u32 + 1, unpaced).0
            });
            match power::played(rng, chances) {
                0 => return None,
                power => power,
            }
        }
        Means::Item { item, .. } => item.casts.power,
        Means::Learned { character, asked } => {
            let affordable = affordable(caster, spell, &content.spells);
            if *asked != power::AUTO {
                return Some(power::chosen(*asked, affordable, spell.power));
            }
            let identify = match (spell.facts.working, cast.choice.aim) {
                (Some(Working::Identify), Aim::Item(target)) => {
                    held.obscurity(target).map(|difficulty| (difficulty, boost))
                }
                _ => None,
            };
            let victim = cast.choice.aim.body().and_then(|aimed| bodies.get(aimed).ok());
            let row = content.spell_row(spell.id);
            let failure = std::array::from_fn(|power| {
                roll::learned(character, spell, power as u32 + 1, pace, false)
            });
            power::auto(
                spell,
                (row.and_then(|row| row.restores), row.map_or(&[], |row| &row.outlasts)),
                victim.as_ref().map(|victim| (victim.vitals, victim.member)),
                affordable,
                failure,
                identify,
            )
        }
    })
}

impl Recast {
    /// Whether to cast again: a living victim is not yet whole in what the
    /// spell restores, and the caster's stamina and points last.
    pub fn wanted(&self, bodies: &Query<BodyReadOnly>, content: &Catalogs) -> bool {
        let Source::Spell { spell, .. } = self.choice.source else { return false };
        let spell = &content.spells[spell];
        let short = |victim: &Entity| {
            bodies.get(*victim).is_ok_and(|body| {
                let pool =
                    if self.watches_stamina { body.vitals.stamina } else { body.vitals.health };
                !body.vitals.is_dead() && pool.current < pool.max
            })
        };
        self.victims.iter().any(short)
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
pub fn resolvable(spell: &SpellDefinition, content: &Catalogs) -> bool {
    content.spell_row(spell.id).is_some() || spell.facts.working.is_some()
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    member: Entity,
    character: &Character,
    held: &Items,
    content: &Catalogs,
) -> Vec<(Entity, SpellId)> {
    held.carried(member)
        .filter_map(|item| {
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            Some((item, usable_item(character, item, held, content, worn)?))
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
    content: &Catalogs,
    worn: bool,
) -> Option<SpellId> {
    let cast = held.casts(item)?;
    (may::activates(cast, worn, character)
        && held.charges(item) != Some(0)
        && resolvable(content.spells.get(cast.spell)?, content))
    .then_some(cast.spell)
}
