//! A monster choosing a spell (`docs/rules/monsters.md` section 5): which of
//! its ten slots it may cast, at whom each may be aimed, whether it casts,
//! which spell, and how hard.
//!
//! A monster spends nothing to cast: its spell budget stands in for a caster's
//! skill in the failure roll, and the power it picks is the most that budget
//! comfortably covers.

use bevy::prelude::*;

use super::sight::Sight;
use super::{ClosesIn, Deliberating, PartyGap};
use crate::content::spells::{MAX_POWER, SpellDefinition, SpellId, Spellbook, Targeting};
use crate::game::actors::Level;
use crate::game::actors::arm::Furthest;
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::combat::turn::{Decided, Decision};
use crate::game::data::Catalogs;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::magic::reach::{Aim, blast_radius, in_cone};
use crate::game::magic::roll::spell_failure;
use crate::game::rng::{GameRng, weighted};
use crate::game::world::hostility::{self, Side, Stance, Standing};
use crate::game::world::identify::GroupMember;
use crate::game::world::population::{SpellBudget, Spellcasting};

pub const SLOT_WEIGHTS: [u32; 10] = [5, 5, 5, 10, 10, 10, 10, 15, 15, 15];

/// Whether the spell may be cast by a monster at all (retail
/// `IsSpellUsableByMonster`): monsters may cast it, the game resolves its
/// effect, it is not aimed at the party, a point, an item or a lock, and a
/// silence does not stop it (an Alchemy spell by a kind that keeps its Alchemy
/// is not stopped).
pub fn castable(
    spell: &SpellDefinition,
    modelled: bool,
    silenced: bool,
    silent_alchemy: bool,
) -> bool {
    spell.monsters_cast
        && modelled
        && !matches!(
            spell.targeting,
            Targeting::Party | Targeting::Point | Targeting::Item | Targeting::LockOrTrap
        )
        && (!silenced || (spell.books.contains(Spellbook::Alchemy) && silent_alchemy))
}

/// A monster considering whom it could act on, and whom it considers: the
/// fight's combatants as it deliberates, or, with no fight on, itself and
/// the party.
#[derive(Component, Clone, Debug)]
pub struct Considers {
    pub by: Entity,
    pub bodies: Vec<Entity>,
}

/// The spell weighed is a breath's cone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Breath;

/// Whether the monster's eyes make out the party at its moment (section 2).
#[derive(Component, Clone, Copy, Debug)]
pub struct SeesParty(pub bool);

/// A spell a monster weighs: which, and the weight of its slot.
#[derive(Component, Clone, Copy, Debug)]
pub struct Weighs {
    pub spell: SpellId,
    pub weight: u32,
}

/// The deliberation, or the look at the party, a spell is weighed in.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Weighing)]
pub struct WeighedIn(pub Entity);

/// The spells a deliberation weighs.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = WeighedIn, linked_spawn)]
pub struct Weighing(Vec<Entity>);

/// At whom a spell weighed may be aimed, each one choice.
#[derive(Component, Clone, Debug)]
pub struct Aims(pub Vec<Aim>);

/// The aims of a spell weighed that leave the neutral alone, judged at a
/// power it might be cast at.
#[derive(Component, Clone, Debug)]
pub struct Spared(pub Vec<Aim>);

/// The deliberation casts none of its spells.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NoSpell;

/// A deliberation picks among the spells it weighed.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct PicksSpell {
    #[event_target]
    deliberation: Entity,
}

/// One a monster weighing its spells has about it: a living body it
/// considers, how it stands toward it (itself a friend), whether it sees it
/// (a member if its eyes make out the party, a monster by a clear line
/// between their eyes, itself always), whether it is a member, and the
/// monster group it is of.
#[derive(Clone, Copy, Debug)]
pub struct Around {
    pub body: Entity,
    pub stance: Stance,
    pub seen: bool,
    pub member: bool,
    pub group: Option<Entity>,
}

/// Whom a monster weighing its spells has about it, in the order it
/// considers them.
#[derive(Component, Clone, Debug)]
pub struct About(pub Vec<Around>);

/// Those about a monster that a spell weighed reaches: within its reach.
#[derive(Component, Clone, Debug)]
pub struct Within(pub Vec<Entity>);

/// Those about a monster a spell weighed would do something to, as its row
/// says (retail `MonsterSpellTargetOK`): a heal the hurt, a refresh the
/// tired, a condition one without it, an enchantment one it is not laid on,
/// a cure one bearing what it cures; a damage spell anyone.
#[derive(Component, Clone, Debug)]
pub struct Feels(pub Vec<Entity>);

/// A spell weighed aimed at its caster.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtCaster;

/// A spell weighed aimed at one body: a friend, or a foe.
#[derive(Component, Clone, Copy, Debug)]
pub struct AtOne(pub Stance);

/// A spell weighed aimed at a group.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtGroup;

/// A spell weighed that is a cone or a blast.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Wide;

/// A spell weighed at all enemies.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtAll;

/// Whom a monster weighing its spells has about it.
pub(super) fn about(
    considered: On<Add<Considers>>,
    mut commands: Commands,
    considering: Query<(&Considers, &SeesParty)>,
    bodies: Query<(&Standing, &Vitals, Option<&GroupMember>)>,
    sight: Sight,
) {
    let entity = considered.entity;
    let Ok((Considers { by: caster, bodies: considers }, &SeesParty(sees_party))) =
        considering.get(entity)
    else {
        return;
    };
    let caster = *caster;
    let Ok((mine, ..)) = bodies.get(caster) else { return };
    let eye = sight.places.of(caster).eye;
    let about = considers
        .iter()
        .filter_map(|&body| {
            let (theirs, vitals, group) = bodies.get(body).ok()?;
            let member = sight.roster().contains(&body);
            let seen = if member {
                sees_party
            } else {
                body == caster || sight.obstacles.clear(eye, sight.places.of(body).eye)
            };
            let stance =
                if body == caster { Stance::Friendly } else { hostility::between(*mine, *theirs) };
            let group = group.map(|group| group.0);
            (!vitals.is_dead()).then_some(Around { body, stance, seen, member, group })
        })
        .collect();
    commands.entity(entity).insert(About(about));
}

/// Those a spell weighed reaches, and the kind of target it is aimed by.
pub(super) fn within(
    weighed: On<Add<WeighedIn>>,
    mut commands: Commands,
    spells: Query<(&Weighs, &WeighedIn)>,
    considering: Query<(&Considers, &About)>,
    places: crate::game::actors::Places,
    content: Res<Catalogs>,
) {
    let child = weighed.entity;
    let Ok((weighs, &WeighedIn(parent))) = spells.get(child) else { return };
    let (Ok((considers, About(about))), Some(spell)) =
        (considering.get(parent), content.spells.get(weighs.spell))
    else {
        return;
    };
    let reach = spell.reach.metres();
    let within =
        about.iter().filter(|it| places.gap(considers.by, it.body) <= reach).map(|it| it.body);
    let mut child = commands.entity(child);
    match spell.targeting {
        Targeting::Caster => child.insert(AtCaster),
        Targeting::Ally => child.insert(AtOne(Stance::Friendly)),
        Targeting::Enemy => child.insert(AtOne(Stance::Hostile)),
        Targeting::EnemyGroup => child.insert(AtGroup),
        Targeting::Cone | Targeting::Radius => child.insert(Wide),
        Targeting::AllEnemies => child.insert(AtAll),
        // Not castable by a monster: nobody to aim at.
        _ => child.insert(Aims(Vec::new())),
    };
    child.insert(Within(within.collect()));
}

/// Those a spell weighed would do something to.
pub(super) fn feels(
    within: On<Add<Within>>,
    mut commands: Commands,
    spells: Query<(&Weighs, &WeighedIn)>,
    considering: Query<&About>,
    bodies: Query<(&Vitals, &Conditions)>,
    laid: crate::game::actors::Laid,
    content: Res<Catalogs>,
) {
    let child = within.entity;
    let Ok((weighs, &WeighedIn(parent))) = spells.get(child) else { return };
    let (Ok(About(about)), Some(spell)) =
        (considering.get(parent), content.spells.get(weighs.spell))
    else {
        return;
    };
    let row = crate::game::hits::definitions::row(spell);
    let feels = about.iter().map(|it| it.body).filter(|&body| {
        let laid = laid.has(body, spell.id);
        row.is_some_and(|row| {
            bodies
                .get(body)
                .is_ok_and(|(vitals, conditions)| row.worthwhile((vitals, conditions), laid))
        })
    });
    commands.entity(child).insert(Feels(feels.collect()));
}

/// Each one seen of this side, members or monsters, reached and feeling it.
fn standing(
    (about, within, feels): (&[Around], &[Entity], &[Entity]),
    members: bool,
    stance: Stance,
) -> impl Iterator<Item = Aim> {
    about
        .iter()
        .filter(move |it| {
            it.seen
                && it.member == members
                && it.stance == stance
                && within.contains(&it.body)
                && feels.contains(&it.body)
        })
        .map(|it| Aim::Body(it.body))
}

/// At least half of the members seen would feel it, and one of them is in
/// reach.
fn half_the_party((about, within, feels): (&[Around], &[Entity], &[Entity])) -> bool {
    let party = about.iter().filter(|it| it.seen && it.member).count();
    let valid = about
        .iter()
        .filter(|it| it.seen && it.member && within.contains(&it.body) && feels.contains(&it.body))
        .count();
    valid != 0 && party.div_ceil(2) <= valid
}

/// The hostile members seen in reach.
fn hostile_members(about: &[Around], within: &[Entity]) -> impl Iterator<Item = Aim> {
    about
        .iter()
        .filter(move |it| {
            it.seen && it.member && within.contains(&it.body) && it.stance == Stance::Hostile
        })
        .map(|it| Aim::Body(it.body))
}

/// A spell aimed at its caster: itself, if it would do something to it.
pub(super) fn aim_at_caster(
    felt: On<Add<Feels>>,
    mut commands: Commands,
    spells: Query<(&WeighedIn, &Feels), With<AtCaster>>,
    considering: Query<(&Considers, &About)>,
) {
    let child = felt.entity;
    let Ok((&WeighedIn(parent), Feels(feels))) = spells.get(child) else { return };
    let Ok((considers, About(about))) = considering.get(parent) else { return };
    let caster = considers.by;
    let alive = about.iter().any(|it| it.body == caster);
    let aims = (alive && feels.contains(&caster)).then_some(Aim::Body(caster));
    commands.entity(child).insert(Aims(aims.into_iter().collect()));
}

/// A spell aimed at a friend or a foe: each one seen and in reach that it
/// would do something to, monsters first and then members.
pub(super) fn aim_at_one(
    felt: On<Add<Feels>>,
    mut commands: Commands,
    spells: Query<(&WeighedIn, &Within, &Feels, &AtOne)>,
    considering: Query<&About>,
) {
    let child = felt.entity;
    let Ok((&WeighedIn(parent), Within(within), Feels(feels), &AtOne(stance))) = spells.get(child)
    else {
        return;
    };
    let Ok(About(about)) = considering.get(parent) else { return };
    let facts = (&about[..], &within[..], &feels[..]);
    let aims = standing(facts, false, stance).chain(standing(facts, true, stance));
    commands.entity(child).insert(Aims(aims.collect()));
}

/// A spell aimed at a group: each other monster group about at least half
/// of whose hostile members would feel it and one of which is in reach,
/// aimed at the first such member, no sight asked; then the party, at its
/// first hostile member in reach, if half of it would feel it.
pub(super) fn aim_at_group(
    felt: On<Add<Feels>>,
    mut commands: Commands,
    spells: Query<(&WeighedIn, &Within, &Feels), With<AtGroup>>,
    considering: Query<(&Considers, &About)>,
) {
    let child = felt.entity;
    let Ok((&WeighedIn(parent), Within(within), Feels(feels))) = spells.get(child) else {
        return;
    };
    let Ok((considers, About(about))) = considering.get(parent) else { return };
    let own = about.iter().find(|it| it.body == considers.by).and_then(|it| it.group);
    let mut groups: Vec<Entity> = Vec::new();
    for group in about.iter().filter_map(|it| it.group) {
        if Some(group) != own && !groups.contains(&group) {
            groups.push(group);
        }
    }
    let mut aims: Vec<Aim> = groups
        .into_iter()
        .filter_map(|group| {
            let foes = || {
                about
                    .iter()
                    .filter(move |it| it.group == Some(group) && it.stance == Stance::Hostile)
            };
            let valid = foes().filter(|it| feels.contains(&it.body)).count();
            let felt = valid != 0 && foes().count().div_ceil(2) <= valid;
            let first = foes().find(|it| within.contains(&it.body));
            first.filter(|_| felt).map(|it| Aim::Body(it.body))
        })
        .collect();
    if half_the_party((about, within, feels)) {
        aims.extend(hostile_members(about, within).next());
    }
    commands.entity(child).insert(Aims(aims));
}

/// A cone or a blast: each foe monster seen and in reach; and each hostile
/// member seen in reach, if half the party would feel it.
pub(super) fn aim_wide(
    felt: On<Add<Feels>>,
    mut commands: Commands,
    spells: Query<(&WeighedIn, &Within, &Feels), With<Wide>>,
    considering: Query<&About>,
) {
    let child = felt.entity;
    let Ok((&WeighedIn(parent), Within(within), Feels(feels))) = spells.get(child) else {
        return;
    };
    let Ok(About(about)) = considering.get(parent) else { return };
    let facts = (&about[..], &within[..], &feels[..]);
    let mut aims: Vec<_> = standing(facts, false, Stance::Hostile).collect();
    if half_the_party(facts) {
        aims.extend(hostile_members(about, within));
    }
    commands.entity(child).insert(Aims(aims));
}

/// A spell at all enemies: ahead, if any foe is seen and in reach.
pub(super) fn aim_at_all(
    felt: On<Add<Feels>>,
    mut commands: Commands,
    spells: Query<(&WeighedIn, &Within, &Feels), With<AtAll>>,
    considering: Query<&About>,
) {
    let child = felt.entity;
    let Ok((&WeighedIn(parent), Within(within), Feels(feels))) = spells.get(child) else {
        return;
    };
    let Ok(About(about)) = considering.get(parent) else { return };
    let facts = (&about[..], &within[..], &feels[..]);
    let foes =
        standing(facts, false, Stance::Hostile).chain(standing(facts, true, Stance::Hostile));
    let aims = if foes.count() != 0 { vec![Aim::Ahead] } else { vec![] };
    commands.entity(child).insert(Aims(aims));
}

/// A deliberation that made no special attack weighs the spells of its slots
/// it may cast now (section 5), unless it closes in: those monsters may cast,
/// the game resolves, not aimed at the party, a point, an item or a lock, and
/// not stopped by a silence (Alchemy by a kind that keeps it is not); then
/// it picks among them.
pub(super) fn weigh_spells(
    weighed: On<Add<super::special::NoSpecial>>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &ClosesIn)>,
    casters: Query<(&Spellcasting, &Conditions)>,
    content: Res<Catalogs>,
) {
    let deliberation = weighed.entity;
    let Ok((&Deliberating(monster), &ClosesIn(closes))) = deliberations.get(deliberation) else {
        return;
    };
    let Ok((casting, conditions)) = casters.get(monster) else { return };
    if closes || casting.chance == 0 {
        commands.entity(deliberation).insert(NoSpell);
        return;
    }
    let silenced = conditions.has(Condition::Silenced);
    for (id, weight) in casting.spells.iter().zip(SLOT_WEIGHTS) {
        let Some(spell) = id.and_then(|id| content.spells.get(id)) else { continue };
        let modelled = crate::game::hits::definitions::row(spell).is_some();
        if castable(spell, modelled, silenced, casting.silent_alchemy) {
            commands.spawn((Weighs { spell: spell.id, weight }, WeighedIn(deliberation)));
        }
    }
    commands.trigger(PicksSpell { deliberation });
}

/// A spell weighed in a deliberation (not a breath's cone, nor a look at the
/// party from outside a fight) keeps the aims that spare the neutral: a cone
/// or a blast at an aim is not cast where it would take in a monster
/// that stands with neither side (retail `SpellAreaHitsNeutralMonster`, which
/// picks a power of its own): a blast within its radius of where it lands, a
/// cone within the spell's reach and its arc, judged at a power the budget
/// might cast it at.
#[allow(clippy::type_complexity)]
pub(super) fn spare_neutrals(
    aimed: On<Add<Aims>>,
    mut commands: Commands,
    spells: Query<(&Weighs, &WeighedIn, &Aims), Without<Breath>>,
    considering: Query<&Considers, With<Deliberating>>,
    budgets: Query<&SpellBudget>,
    neutrals: Query<(Entity, &Side, &Vitals)>,
    sight: Sight,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) {
    let child = aimed.entity;
    let Ok((weighs, &WeighedIn(deliberation), Aims(aims))) = spells.get(child) else { return };
    let Ok(&Considers { by: caster, .. }) = considering.get(deliberation) else { return };
    let Some(spell) = content.spells.get(weighs.spell) else { return };
    let budget = budgets.get(caster).map_or(0, |budget| budget.0);
    let at = power(&mut rng, budget, spell);
    let eye = sight.places.of(caster).eye;
    let neutral: Vec<Entity> = neutrals
        .iter()
        .filter(|&(body, side, vitals)| {
            body != caster
                && !sight.roster().contains(&body)
                && !vitals.is_dead()
                && side.own() == Stance::Neutral
        })
        .map(|(body, ..)| body)
        .collect();
    let spares = |aim: &Aim| {
        let Aim::Body(target) = *aim else { return true };
        let target_at = sight.places.of(target);
        let mut around = neutral.iter().map(|&body| (body, sight.places.of(body)));
        let hits = match spell.targeting {
            Targeting::Radius => {
                let radius = blast_radius(spell, at);
                around.any(|(_, there)| {
                    there.centre.distance(target_at.feet) - there.radius <= radius
                        && sight.obstacles.clear(target_at.feet, there.centre)
                })
            }
            Targeting::Cone => {
                let toward = target_at.feet - eye;
                around.any(|(body, there)| {
                    sight.places.gap(caster, body) <= spell.reach.metres()
                        && in_cone(toward, there.feet - eye)
                })
            }
            _ => false,
        };
        !hits
    };
    commands.entity(child).insert(Spared(aims.iter().copied().filter(spares).collect()));
}

/// Whether it casts, and what (section 5): its chance (the species' while
/// one of its attacks reaches the party, 100 when none does), rolled once if
/// any spell weighed has an aim; then one of those spells by the weights of
/// their slots, at one of its aims at random, at the power its budget covers.
#[allow(clippy::type_complexity)]
fn pick(
    picks: On<PicksSpell>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap, Option<&Weighing>)>,
    spells: Query<(&Weighs, &Spared)>,
    casters: Query<(&Spellcasting, &Furthest, &Level, Option<&SpellBudget>)>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) {
    let deliberation = picks.deliberation;
    let Ok((&Deliberating(monster), &PartyGap(gap), weighing)) = deliberations.get(deliberation)
    else {
        commands.entity(deliberation).insert(NoSpell);
        return;
    };
    let Ok((casting, reach, level, budget)) = casters.get(monster) else { return };
    // A monster with nothing castable weighed nothing.
    let weighed = weighing.map_or(&[][..], |weighing| &weighing.0[..]);
    let usable: Vec<(&Weighs, &Vec<Aim>)> = weighed
        .iter()
        .filter_map(|&child| spells.get(child).ok())
        .filter(|(_, spared)| !spared.0.is_empty())
        .map(|(weighs, spared)| (weighs, &spared.0))
        .collect();
    let chance = if reach.reaches(gap) { u32::from(casting.chance) } else { 100 };
    let Some(budget) = budget.map(|budget| budget.0) else {
        commands.entity(deliberation).insert(NoSpell);
        return;
    };
    if usable.is_empty() || rng.below(100) >= chance {
        commands.entity(deliberation).insert(NoSpell);
        return;
    }
    let total: u32 = usable.iter().map(|(weighs, _)| weighs.weight).sum();
    let which = weighted(rng.below(total), usable.iter().map(|(weighs, _)| weighs.weight))
        .unwrap_or(usable.len() - 1);
    let (weighs, aims) = usable[which];
    let aim = aims[rng.below(aims.len() as u32) as usize];
    let power = power(&mut rng, budget, &content.spells[weighs.spell]);
    let source = Source::Innate { spell: weighs.spell, power, budget, level: level.0 as u8 };
    commands.trigger(Decided {
        actor: monster,
        decision: Decision::Magic(MagicChoice { source, aim }),
        own_will: true,
    });
    commands.entity(deliberation).despawn();
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(about)
        .add_observer(within)
        .add_observer(feels)
        .add_observer(aim_at_caster)
        .add_observer(aim_at_one)
        .add_observer(aim_at_group)
        .add_observer(aim_wide)
        .add_observer(aim_at_all)
        .add_observer(weigh_spells)
        .add_observer(spare_neutrals)
        .add_observer(pick);
}

/// How hard it casts (retail `ChooseMonsterSpellPowerLevel`): up from 1 while
/// the spell's failure chance at that power with the budget as skill is under 9
/// percent and the power is under 8; then one less if above 1; then below(4): on
/// 0 one less if above 2, on 1 one more if under 7.
pub fn power(rng: &mut GameRng, budget: u32, spell: &SpellDefinition) -> u32 {
    let mut power = 1;
    while power < 8 && spell_failure(budget, spell, power) < 9 {
        power += 1;
    }
    if power > 1 {
        power -= 1;
    }
    match rng.below(4) {
        0 if power > 2 => power - 1,
        1 if power < MAX_POWER => power + 1,
        _ => power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::Reach;

    fn spell(id: u8, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId(id),
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            reach: Reach::Long,
            ..Default::default()
        }
    }

    // Power: failure of the test spell (cost 2, level 1: band 2, threshold 50) at
    // power p is need = 50 * p / 7 = 7, 14, 21, 28, 35, 42, 50 for 1 to 7, and
    // (need - budget) * 70 / need when the budget is short of it.

    fn powers(budget: u32) -> Vec<u32> {
        let blast = spell(1, Targeting::Enemy);
        let mut rng = GameRng::seeded(4);
        let mut got: Vec<u32> = (0..200).map(|_| power(&mut rng, budget, &blast)).collect();
        got.sort_unstable();
        got.dedup();
        got
    }

    #[test]
    fn a_middling_budget_casts_one_below_the_last_power_it_covers_give_or_take_one() {
        // Budget 30: powers 1-4 need 7, 14, 21, 28, all met; power 5 needs 35:
        // (35 - 30) * 70 / 35 = 10, not under 9. So 5, then 4; roll 0 -> 3,
        // roll 1 -> 5, else 4.
        assert_eq!(powers(30), [3, 4, 5]);
        // Budget 31: (35 - 31) * 70 / 35 = 8 is under 9: power 6 is tried; it
        // needs 42: (42 - 31) * 70 / 42 = 18: stops at 6, then 5; 4, 5 or 6.
        assert_eq!(powers(31), [4, 5, 6]);
    }
}
