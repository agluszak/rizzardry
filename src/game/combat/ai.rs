//! A body deciding its action for itself (retail `UpdateMonsterAI`,
//! `BuildMonsterActionQueue`): to run, withdraw or hold back by its
//! [`Temper`], to make its [`Special`] attack, to cast by its
//! [`Spellcasting`], to withdraw when it can strike nobody, to guard a
//! fellow, to hold back warily, or to fight; the first that applies. What
//! it decides from is what it sees of the fight ([`seen`]) and the facts
//! judged on everyone.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::conduct;
use super::monster_spells;
use super::turn::{Breathes, Chosen, Fights, Fleeing, Retreats, SpecialCooldown};
use crate::content::characters::Attribute;
use crate::content::species::{Reach, SpecialAttack};
use crate::content::spells::SpellId;
use crate::game::actors::Attacks as Arms;
use crate::game::actors::{Eyes, Hidden, Laid, Places};
use crate::game::characters::{Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::BodyReadOnly;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::npcs::NpcBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Loyalty, Stance};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Special, Spellcasting, Temper};

/// Whether a body can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it. A summoning is never what
/// starts a fight (`fighting` is `None` then, retail's `exclude_special`);
/// in one it needs the party within long reach and the summoner not on its
/// side. Any other kind also needs an aim, which its caller finds.
pub(super) fn special_ready(
    vitals: &Vitals,
    special: &SpecialAttack,
    cooling: bool,
    fighting: Option<(f32, Stance)>,
) -> bool {
    let silenced = vitals.conditions.has(Condition::Silenced);
    special.kind.is_some_and(|kind| {
        let definition = crate::game::hits::specials::special(kind);
        let summons = kind.summons().is_none()
            || fighting.is_some_and(|(nearest, stance)| {
                nearest <= Reach::Long.metres() && stance != Stance::Friendly
            });
        !cooling
            && vitals.stamina.current >= vitals.stamina.max / 10
            && !(silenced && definition.silenced)
            && summons
    })
}

/// Whether a body's eyes make out the party: the party hides by its
/// lowest able level and by Chameleon.
pub(super) fn sees_party(
    body: Entity,
    (bodies, places, laid): (&Query<BodyReadOnly>, &Places, &Laid),
    (party, roster): (Entity, &PartyRoster),
    far: f32,
    obstacles: &WorldObstacles,
) -> bool {
    let Ok(own) = bodies.get(body) else { return false };
    let members =
        bodies.iter_many(roster.iter()).matched().map(|member| (member.level, member.vitals));
    let hidden = Hidden::party(members, laid.bonus(party).concealment);
    Eyes::of(own.vitals, own.attributes, own.attacks).see(
        &places.of(body),
        (&places.of(party), hidden),
        far,
        obstacles,
    )
}

/// Whom a body sees of the fight (retail `IsVisibleUnderConditions`,
/// `CanMonsterSeeMonster`): the living party when its eyes see the party,
/// and the living monsters in the fight it has a clear line to, itself
/// among them.
pub(super) fn seen(body: Entity, fight: &Fight) -> Vec<Entity> {
    let Fight { bodies, places, laid, party, roster, combatants, far, obstacles, .. } = *fight;
    let sees_party = sees_party(body, (bodies, places, laid), (party, roster), far, obstacles);
    let eye = places.of(body).eye;
    combatants
        .iter()
        .copied()
        .filter(|&other| {
            if roster.contains(&other) {
                sees_party
            } else {
                other == body || obstacles.clear(eye, places.of(other).eye)
            }
        })
        .filter(|&other| bodies.get(other).is_ok_and(|body| !body.vitals.is_dead()))
        .collect()
}

/// Whether one of a body's arms reaches the party.
fn reaches_party(arms: &Arms, nearest: f32) -> bool {
    arms.arms.iter().any(|arm| nearest <= arm.reach.metres())
}

/// Whether a body would close in to strike rather than cast or breathe:
/// it prefers blows, has some, and none reaches the party.
fn closes_in(temper: &Temper, arms: &Arms, nearest: f32) -> bool {
    !temper.0.prefers_range && !arms.arms.is_empty() && !reaches_party(arms, nearest)
}

/// What a monster decides by, besides what every body in a fight has.
#[derive(QueryData)]
pub(super) struct Mind {
    temper: &'static Temper,
    special: &'static Special,
    casting: &'static Spellcasting,
    cooldown: Option<&'static SpecialCooldown>,
    npc: Has<NpcBody>,
    pub fleeing: Has<Fleeing>,
}

/// What the deciding body weighs its choice in.
pub(super) struct Fight<'a, 'w, 's> {
    pub bodies: &'a Query<'w, 's, BodyReadOnly>,
    pub laid: &'a Laid<'w, 's>,
    pub places: &'a Places<'w, 's>,
    /// The party, which every member stands in.
    pub party: Entity,
    pub roster: &'a PartyRoster,
    pub combatants: &'a [Entity],
    /// The far clip, past which nobody sees anybody.
    pub far: f32,
    pub obstacles: &'a WorldObstacles<'w, 's>,
    pub content: &'a Catalogs,
    pub definitions: &'a Definitions,
}

/// What a body deciding for itself does with its turn, and whether it is
/// in flight from now on.
pub(super) fn decide(
    body: Entity,
    mind: &MindItem,
    fight: &Fight,
    rng: &mut GameRng,
) -> (Chosen, bool) {
    let Fight { bodies, places, .. } = *fight;
    let own = bodies.get(body).unwrap();
    let arms = own.attacks;
    let nearest = places.gap(body, fight.party);
    // Run, withdraw or hold back, as it stands and fares (retail
    // `UpdateMonsterAI`'s flight test); a flight begun goes on in later
    // rounds while the party stays off.
    let mut fleeing = mind.fleeing;
    let conduct = conduct::decide(
        rng,
        own.vitals,
        &mind.temper.0,
        (mind.npc, *own.loyalty == Loyalty::Bystander),
        own.stance(),
        &mut fleeing,
        nearest,
    );
    if conduct != conduct::Conduct::Fight {
        return (Chosen::Retreat(Retreats { conduct, quiet: false }), fleeing);
    }
    let chosen = special(body, mind, arms, nearest, fight, rng)
        .or_else(|| spell(body, mind, arms, nearest, fight, rng))
        .or_else(|| helpless(own.vitals, arms, nearest))
        .or_else(|| guard(body, mind, nearest, fight, rng))
        .unwrap_or(Chosen::Fight(Fights { waits: false, berserk: false }));
    (chosen, fleeing)
}

/// A body with a special attack ready may make it (retail
/// `UpdateMonsterAI`'s special branch): always if it cannot strike the
/// party, else at its chance; one that would close in to strike does that
/// instead. A breath is aimed at one of its aims at random (retail
/// `AimMonsterAtSpellTarget`) and catches whoever stands in its cone; a
/// summoning needs no one in sight.
fn special(
    body: Entity,
    mind: &MindItem,
    arms: &Arms,
    nearest: f32,
    fight: &Fight,
    rng: &mut GameRng,
) -> Option<Chosen> {
    let Fight { bodies, content, definitions, .. } = *fight;
    let own = bodies.get(body).ok()?;
    let special = mind.special.0;
    let kind = special.kind?;
    let cooling = mind.cooldown.is_some_and(|cooldown| cooldown.0 > 0);
    if special.chance == 0
        || closes_in(mind.temper, arms, nearest)
        || !special_ready(own.vitals, &special, cooling, Some((nearest, own.stance())))
    {
        return None;
    }
    let toward = if kind.summons().is_some() {
        fight.places.of(body).feet
    } else {
        let cone = content.spells.get(SpellId::SPECIAL_ATTACK_CONE)?;
        let seen = seen(body, fight);
        let aims = monster_spells::aims(cone, body, &seen, fight);
        if aims.is_empty() {
            return None;
        }
        let aim = aims[rng.below(aims.len() as u32) as usize];
        aim.body()
            .and_then(|aimed| bodies.get(aimed).ok())
            .map_or(fight.places.of(body).feet, |there| fight.places.of(there.entity).feet)
    };
    let chance = if reaches_party(arms, nearest) { u32::from(special.chance) } else { 100 };
    if rng.below(100) >= chance {
        return None;
    }
    Some(Chosen::Special(Breathes {
        special: Some(kind),
        definition: definitions.special(kind),
        summons: kind.summons(),
        strength: crate::game::hits::specials::strength(own.level.0 as u8, own.vitals.stamina),
        toward,
        cost: own.vitals.stamina.max / 10,
        cooldown: special.cooldown,
    }))
}

/// A body that casts may cast (retail `UpdateMonsterAI`'s spell branch):
/// always if it cannot strike the party, else at its chance; one that would
/// close in to strike does that instead.
fn spell(
    body: Entity,
    mind: &MindItem,
    arms: &Arms,
    nearest: f32,
    fight: &Fight,
    rng: &mut GameRng,
) -> Option<Chosen> {
    let Fight { bodies, content, .. } = *fight;
    let own = bodies.get(body).ok()?;
    let budget = own.budget?.0;
    let casting = mind.casting;
    if casting.chance == 0 || closes_in(mind.temper, arms, nearest) {
        return None;
    }
    let chance = if reaches_party(arms, nearest) { u32::from(casting.chance) } else { 100 };
    let seen = seen(body, fight);
    let (spell, aim) = monster_spells::choose(rng, body, casting, chance, &seen, fight)?;
    let power = monster_spells::power(rng, budget, &content.spells[spell]);
    let source = Source::Innate { spell, power, budget, level: own.level.0 as u8 };
    Some(Chosen::Magic(MagicChoice { source, aim }))
}

/// One that can strike nobody, having no arm or, blind, nobody in reach,
/// withdraws (retail `UpdateMonsterAI`'s `RETURN_TO_START`); one that
/// fights blind fights on.
fn helpless(vitals: &Vitals, arms: &Arms, nearest: f32) -> Option<Chosen> {
    let blind = vitals.conditions.has(Condition::Blind) && arms.blind_fighting.is_none();
    (arms.arms.is_empty() || (blind && !reaches_party(arms, nearest)))
        .then_some(Chosen::Retreat(Retreats { conduct: conduct::Conduct::Withdraw, quiet: false }))
}

/// A bodyguard with someone in reach, as clever as it is, may guard a
/// stronger fellow in its reach in trouble (retail
/// `BuildMonsterActionQueue`); a wary one may hold back instead.
fn guard(
    body: Entity,
    mind: &MindItem,
    nearest: f32,
    fight: &Fight,
    rng: &mut GameRng,
) -> Option<Chosen> {
    let Fight { bodies, combatants, .. } = *fight;
    let own = bodies.get(body).ok()?;
    let (arms, wit) = (own.attacks, own.attributes[Attribute::Intelligence]);
    let temper = mind.temper.0;
    if !reaches_party(arms, nearest) {
        return None;
    }
    let able = temper.bodyguard && own.vitals.can_guard() && !arms.arms.is_empty();
    let mine = own.standing();
    let wards: Vec<Entity> = if able && rng.below(100) < wit {
        let reach = arms.arms[0].reach.metres();
        combatants
            .iter()
            .copied()
            .filter(|&other| other != body)
            .filter(|&other| {
                bodies.get(other).is_ok_and(|fellow| {
                    !fellow.vitals.is_dead()
                        && hostility::between(mine, fellow.standing()) == Stance::Friendly
                        && fight.places.gap(body, fellow.entity) <= reach
                        && fellow.level > own.level
                        && fellow.vitals.health.current * 100 / fellow.vitals.health.max < 40
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    if !wards.is_empty() {
        Some(Chosen::Protect(wards[rng.below(wards.len() as u32) as usize]))
    } else if temper.wary && rng.below(100) < wit {
        Some(Chosen::Fight(Fights { waits: true, berserk: false }))
    } else {
        None
    }
}
