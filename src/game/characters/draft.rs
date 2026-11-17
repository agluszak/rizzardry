//! A character being made, or what a member is to become
//! (`docs/rules/characters.md`, "How this is built"). A draft is an actor:
//! the sources a member has, its figures written as any actor's are, and
//! what may be spent on it as components of its own. Spending is an event;
//! accepting writes what it became onto its member.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::progression::SpellPicks;
use super::{Birth, Calling, Experience, Identity, Profession, Training, Vitals};
use crate::content::characters::{Attribute, Skill};
use crate::content::spells::SpellId;
use crate::game::actors::{Abilities, Skills, UsableSkills};
use crate::game::data::Catalogs;
use crate::game::magic;
use crate::game::magic::castable::{CastableOf, CastableSpell, Castables, Learned, learned};

/// One being made, or what a member is to become: no one yet, and taken in
/// by nothing of play.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Drafted;

/// The member a level-up draft is to become; it goes with them.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Becoming)]
pub struct Becomes(pub Entity);

/// What a member is to become, while they choose.
#[derive(Component, Debug)]
#[relationship_target(relationship = Becomes, linked_spawn)]
pub struct Becoming(Entity);

/// The points granted to spend: 60 less what the profession's minimums cost
/// and 15 to make a character, 6 and 9 for a level.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Points {
    pub attributes: u32,
    pub skills: u32,
}

/// What the rules raised an attribute by for free, out of the points: a new
/// profession's minimums at a level.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Forced(pub EnumMap<Attribute, u32>);

/// What the player put into each attribute and skill.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Allotted {
    pub attributes: EnumMap<Attribute, u32>,
    pub skills: EnumMap<Skill, u32>,
}

/// The spells picked, in the order they were picked.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Picked(pub Vec<SpellId>);

/// The most an attribute or a skill takes of a pool: a third of it rounded
/// up, at least 3, never more than the pool (retail `RecomputeAttributeLimits`,
/// `RecomputeSkillLimits`).
pub fn step(pool: u32) -> u32 {
    pool.div_ceil(3).max(3).min(pool)
}

/// The attribute points left: the pool less what the rules raised and what
/// was put in.
pub fn attributes_left(points: &Points, forced: &Forced, allotted: &Allotted) -> u32 {
    points.attributes - forced.0.values().sum::<u32>() - allotted.attributes.values().sum::<u32>()
}

/// The skill points left.
pub fn skills_left(points: &Points, allotted: &Allotted) -> u32 {
    points.skills - allotted.skills.values().sum::<u32>()
}

/// Whether an attribute may take another point: points are left, it is under
/// 100, and what went into it is under its step.
pub fn may_raise_attribute(
    (points, forced, allotted): (&Points, &Forced, &Allotted),
    abilities: &Abilities,
    attribute: Attribute,
) -> bool {
    attributes_left(points, forced, allotted) > 0
        && abilities.0[attribute] < 100
        && forced.0[attribute] + allotted.attributes[attribute] < step(points.attributes)
}

/// Whether a skill may take another point: points are left, it is under 75,
/// what went into it is under its step, and the character may use it.
pub fn may_raise_skill(
    (points, allotted): (&Points, &Allotted),
    (training, usable): (&Training, &UsableSkills),
    skill: Skill,
) -> bool {
    skills_left(points, allotted) > 0
        && training.0[skill] < 75
        && allotted.skills[skill] < step(points.skills)
        && usable.0.contains(skill)
}

/// What is spent on a draft, up or down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allotment {
    Attribute(Attribute),
    Skill(Skill),
}

/// A point is put into, or taken back from, an attribute or a skill.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Allot {
    #[event_target]
    pub draft: Entity,
    pub what: Allotment,
    pub up: bool,
}

/// A spell is picked, or given back.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Pick {
    #[event_target]
    pub draft: Entity,
    pub spell: SpellId,
}

/// The profession a draft is to follow.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Follow {
    #[event_target]
    pub draft: Entity,
    pub profession: Profession,
}

/// A point goes in or comes back, within its limits.
fn allot(
    allot: On<Allot>,
    mut drafts: Query<(
        &Points,
        &Forced,
        &mut Allotted,
        &mut Abilities,
        &mut Training,
        &UsableSkills,
    )>,
) -> Result {
    let (points, forced, mut allotted, mut abilities, mut training, usable) =
        drafts.get_mut(allot.draft)?;
    match (allot.what, allot.up) {
        (Allotment::Attribute(a), true) => {
            if may_raise_attribute((points, forced, &allotted), &abilities, a) {
                allotted.attributes[a] += 1;
                abilities.0[a] += 1;
            }
        }
        (Allotment::Attribute(a), false) => {
            if allotted.attributes[a] > 0 {
                allotted.attributes[a] -= 1;
                abilities.0[a] -= 1;
            }
        }
        (Allotment::Skill(s), true) => {
            if may_raise_skill((points, &allotted), (&training, usable), s) {
                allotted.skills[s] += 1;
                training.0[s] += 1;
            }
        }
        (Allotment::Skill(s), false) => {
            if allotted.skills[s] > 0 {
                allotted.skills[s] -= 1;
                training.0[s] -= 1;
            }
        }
    }
    Ok(())
}

/// A skill a draft may no longer use gives back what was put into it: an
/// expert skill whose attribute fell under 100, a realm skill whose last
/// spell was given back (retail `RefreshCharacterSkillAvailability`,
/// `RefundSkillAllocation`).
pub fn give_back(
    mut drafts: Query<
        (&UsableSkills, &mut Allotted, &mut Training),
        (With<Drafted>, Changed<UsableSkills>),
    >,
) {
    for (usable, mut allotted, mut training) in &mut drafts {
        for skill in EnumSet::<Skill>::all().iter().filter(|&skill| !usable.0.contains(skill)) {
            let put = allotted.skills[skill];
            if put > 0 {
                allotted.skills[skill] = 0;
                training.0[skill] -= put;
            }
        }
    }
}

/// Whether a member may take up a new profession at a level: one of their
/// gender, whose minimums their attributes reach within 3 each and 6 in all
/// (retail `DetermineEligibleProfessions`); their own always.
pub fn may_follow(
    profession: Profession,
    (birth, abilities, calling): (&Birth, &Abilities, &Calling),
) -> bool {
    if profession == calling.profession {
        return true;
    }
    if profession.row().only_gender.is_some_and(|only| only != birth.gender) {
        return false;
    }
    let deficits = profession.row().minimums.iter().map(|(a, &n)| n - n.min(abilities.0[a]));
    let deficits: Vec<u32> = deficits.collect();
    deficits.iter().all(|&n| n <= 3) && deficits.iter().sum::<u32>() <= 6
}

/// A member gains a level: what they are to become is a draft of them, the
/// level gained in the profession they follow, until they accept it or let
/// it go.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Advance {
    #[event_target]
    pub member: Entity,
}

/// What may keep a member from gaining a level now: a fight, a conversation,
/// travel, not being in the party, or not being ready.
pub fn may_advance(
    busy: bool,
    seated: bool,
    (calling, experience, vitals): (&Calling, &Experience, &Vitals),
) -> Result<(), &'static str> {
    if busy {
        return Err("Finish the current encounter or journey first.");
    }
    if !seated {
        return Err("This character is no longer in the party.");
    }
    if !super::progression::ready(calling, experience, vitals) {
        return Err("This character is not ready to gain a level.");
    }
    Ok(())
}

/// The draft of a member gaining a level: a copy of what they are, made to
/// follow their profession.
#[allow(clippy::type_complexity)]
fn advance(
    advance: On<Advance>,
    members: Query<(
        &Calling,
        &Experience,
        &Vitals,
        Has<crate::game::party::roster::PartyMember>,
        Has<Becoming>,
    )>,
    busy: Query<
        (),
        Or<(
            With<crate::game::world::level::Destination>,
            With<crate::game::npcs::conversation::Conversation>,
            With<crate::game::world::level::TravelQuestion>,
        )>,
    >,
    battle: Option<Res<crate::game::combat::Battle>>,
    known: (Query<&Castables>, Query<&CastableSpell, With<Learned>>),
    mut commands: Commands,
) -> Result {
    let member = advance.member;
    let (calling, experience, vitals, seated, choosing) = members.get(member)?;
    let busy = battle.is_some() || !busy.is_empty();
    if choosing || may_advance(busy, seated, (calling, experience, vitals)).is_err() {
        return Ok(());
    }
    let profession = calling.profession;
    let draft = commands
        .entity(member)
        .clone_and_spawn_with_opt_in(|builder| {
            builder.allow::<(
                Identity,
                Birth,
                Calling,
                Abilities,
                Training,
                SpellPicks,
                Experience,
                Vitals,
            )>();
        })
        .insert((Drafted, Becomes(member)))
        .insert((Forced::default(), Allotted::default(), Picked::default()))
        .observe(follow)
        .observe(pick_learning)
        .observe(accept)
        .id();
    // What they know, the draft knows.
    let (owned, spells) = known;
    for CastableSpell(spell) in
        spells.iter_many(owned.get(member).into_iter().flat_map(|o| o.iter())).flatten()
    {
        commands.spawn((CastableOf(draft), learned(spell)));
    }
    commands.trigger(Follow { draft, profession });
    Ok(())
}

/// A level-up draft follows a profession: what was spent comes back, and it
/// is laid out again from its member (retail `InitializeCharacterLevelUp`,
/// `RebuildLevelUpPoolsForProfession`): a level more in the profession, the
/// next goal from the one passed, attributes raised to its minimums out of
/// the points, a new profession's skills and bonus skill raised to 5 for
/// free, and its spellbook skills too once its caster level reaches 1; a
/// pick more at caster level 1 or more.
#[allow(clippy::type_complexity)]
fn follow(
    follow: On<Follow>,
    drafts: Query<&Becomes>,
    members: Query<
        (&Birth, &Calling, &Abilities, &Training, &SpellPicks, &Experience),
        Without<Becomes>,
    >,
    mut commands: Commands,
) -> Result {
    let Follow { draft, profession } = *follow;
    let &Becomes(member) = drafts.get(draft)?;
    let (birth, was, abilities, training, picks, experience) = members.get(member)?;
    if !may_follow(profession, (birth, abilities, was)) {
        return Ok(());
    }
    let mut calling = was.clone();
    calling.profession = profession;
    calling.levels[profession] += 1;
    let experience = Experience { last_goal: was.experience_goal(experience), ..*experience };
    let mut abilities = *abilities;
    let mut points = Points { attributes: 6, skills: 9 };
    let forced = match profession == was.profession {
        true => {
            points.attributes -= pay_down_debt(&mut abilities, profession, points.attributes);
            Forced::default()
        }
        false => raise_to_minimums(&mut abilities, profession),
    };
    let mut training = training.clone();
    if profession != was.profession {
        let row = profession.row();
        for &s in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            training.0[s] = training.0[s].max(5);
        }
    }
    let caster = magic::caster_level(&calling, profession);
    if caster == 1 && magic::caster_level(was, profession) < 1 {
        for book in profession.row().books {
            let s = magic::book_skill(book);
            training.0[s] = training.0[s].max(5);
        }
    }
    let mut picks = picks.clone();
    if caster > 0 {
        picks.grant(profession, 1);
    }
    commands.entity(draft).insert((
        calling,
        experience,
        abilities,
        training,
        picks,
        points,
        forced,
        Allotted::default(),
        Picked::default(),
    ));
    Ok(())
}

/// Attributes fallen below one's own profession's minimums are a debt, paid
/// down out of a level's points a point at a time, the largest deficit first,
/// as far as the points go (retail `PayDownAttributeDebt`, CG:371-417). The
/// points it took.
fn pay_down_debt(abilities: &mut Abilities, profession: Profession, points: u32) -> u32 {
    let minimums = &profession.row().minimums;
    let mut paid = 0;
    while paid < points {
        let deficit = |a: Attribute| minimums[a] - minimums[a].min(abilities.0[a]);
        let Some(worst) = EnumSet::<Attribute>::all()
            .iter()
            .filter(|&a| deficit(a) > 0)
            .max_by_key(|&a| (deficit(a), std::cmp::Reverse(a as u8)))
        else {
            break;
        };
        abilities.0[worst] += 1;
        paid += 1;
    }
    paid
}

/// A new profession's minimums, raised out of the points (retail
/// `ApplyProfessionMinimumAttributes`): what each raise took.
fn raise_to_minimums(abilities: &mut Abilities, profession: Profession) -> Forced {
    let mut forced = Forced::default();
    for (a, &minimum) in &profession.row().minimums {
        forced.0[a] = minimum - minimum.min(abilities.0[a]);
        abilities.0[a] = abilities.0[a].max(minimum);
    }
    forced
}

/// The spell a pick holds it under: Set Portal brings Return to Portal for
/// one pick (retail `SelectCreationSpell`).
fn pick_key(id: SpellId) -> SpellId {
    if id == SpellId::RETURN_TO_PORTAL { SpellId::SET_PORTAL } else { id }
}

/// What picks are left of those granted once the spells picked are charged
/// to their books; none if they cannot all be.
pub fn picks_left(
    granted: &SpellPicks,
    calling: &Calling,
    picked: &[SpellId],
    content: &Catalogs,
) -> Option<SpellPicks> {
    let mut picks = granted.clone();
    for &id in picked {
        let books = content.spells.get(id)?.books;
        if !picks.spend(calling, books) {
            return None;
        }
    }
    Some(picks)
}

/// What a level-up draft's skills were before this level's points: what
/// learning a spell is judged by.
pub fn before_this_level(training: &Training, allotted: &Allotted) -> Training {
    let mut before = training.clone();
    for (skill, &put) in &allotted.skills {
        before.0[skill] -= put;
    }
    before
}

/// Whether one gaining a level may learn a spell they do not know: one
/// their caster level and their realm and book skills reach, as they were
/// before this level's skill points (`training`; retail
/// `CountRemainingSpellPoints` takes the draft's own off) with the best
/// book skill as it stands (`standing`), and the draft's caster levels for
/// it (`caster`: the followed profession's, and in all for the spell's books).
pub fn may_learn(
    (calling, traits, training): (&Calling, EnumSet<crate::content::characters::Trait>, &Training),
    standing: &Skills,
    caster: (i32, i32),
    spell: &crate::content::spells::SpellDefinition,
) -> bool {
    magic::can_learn((calling, traits, training), standing, caster, spell)
}

/// A member gaining a level picks a spell they do not know that they may
/// learn now, judged by what they were before this level's skill points
/// (retail `CountRemainingSpellPoints`), while picks are left; one picked is
/// given back. What they know follows ([`picks_known`]).
#[allow(clippy::type_complexity)]
fn pick_learning(
    pick: On<Pick>,
    mut drafts: Query<(
        &Becomes,
        &Calling,
        (&crate::game::actors::Traits, &crate::game::actors::CasterLevels),
        &Training,
        &Allotted,
        &SpellPicks,
        &mut Picked,
    )>,
    members: Query<(Option<&Castables>, &Skills), Without<Becomes>>,
    known: Query<&CastableSpell, With<Learned>>,
    content: Res<Catalogs>,
) -> Result {
    let (&Becomes(member), calling, (traits, levels), training, allotted, granted, mut picked) =
        drafts.get_mut(pick.draft)?;
    let (owned, standing) = members.get(member)?;
    let id = pick_key(pick.spell);
    if let Some(at) = picked.0.iter().position(|&spell| spell == id) {
        picked.0.remove(at);
        return Ok(());
    }
    let owned = owned.into_iter().flat_map(|owned| owned.iter());
    let knows = known.iter_many(owned).flatten().any(|spell| spell.0.id == id);
    let before = before_this_level(training, allotted);
    let learnable = !knows
        && content.spells.get(id).is_some_and(|spell| {
            let caster = (levels.followed, levels.caster_level(spell));
            may_learn((calling, traits.0, &before), standing, caster, spell)
        });
    let mut more = picked.0.clone();
    more.push(id);
    if learnable && picks_left(granted, calling, &more, &content).is_some() {
        picked.0 = more;
    }
    Ok(())
}

/// A spell picked on a draft is known to it while it is picked: Set Portal
/// brings Return to Portal with it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PickedSpell;

/// What a draft knows follows what is picked on it.
pub fn picks_known(
    drafts: Query<(Entity, &Picked, Option<&Castables>), Changed<Picked>>,
    picks: Query<(), With<PickedSpell>>,
    content: If<Res<Catalogs>>,
    mut commands: Commands,
) {
    for (draft, picked, owned) in &drafts {
        for castable in owned.into_iter().flat_map(|owned| owned.iter()) {
            if picks.contains(castable) {
                commands.entity(castable).despawn();
            }
        }
        let portal = picked.0.contains(&SpellId::SET_PORTAL).then_some(SpellId::RETURN_TO_PORTAL);
        for id in picked.0.iter().copied().chain(portal) {
            let spell = &content.spells[id];
            commands.spawn((CastableOf(draft), learned(spell), PickedSpell));
        }
    }
}

/// A level-up draft is accepted.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Accept {
    #[event_target]
    pub draft: Entity,
}

/// Why a level-up draft may not be accepted yet.
pub fn unfinished(
    (points, forced, allotted): (&Points, &Forced, &Allotted),
    (abilities, training, usable): (&Abilities, &Training, &UsableSkills),
) -> Option<&'static str> {
    if EnumSet::<Attribute>::all()
        .iter()
        .any(|a| may_raise_attribute((points, forced, allotted), abilities, a))
    {
        return Some("Allocate the remaining attribute points.");
    }
    if EnumSet::<Skill>::all()
        .iter()
        .any(|s| may_raise_skill((points, allotted), (training, usable), s))
    {
        return Some("Allocate the remaining skill points.");
    }
    None
}

/// Accepting a level writes what the draft became onto its member: the
/// level, the profession, the attributes, the skills, the picks left and the
/// spells. Health, stamina, points and armour follow as figures do, with no
/// healing; a Gadgeteer's omnigun follows the level. The draft is gone.
#[allow(clippy::type_complexity)]
fn accept(
    accept: On<Accept>,
    drafts: Query<(
        &Becomes,
        (&Points, &Forced, &Allotted, &UsableSkills),
        (&Calling, &Experience, &Abilities, &Training, &SpellPicks, &Picked),
        Option<&Castables>,
    )>,
    picked_spells: Query<(), With<PickedSpell>>,
    mut members: Query<
        (&mut Calling, &mut Experience, &mut Abilities, &mut Training, &mut SpellPicks),
        (Without<Drafted>, Without<Becomes>),
    >,
    content: Res<Catalogs>,
    mut commands: Commands,
) -> Result {
    let (
        &Becomes(member),
        (points, forced, allotted, usable),
        (calling, experience, abilities, training, granted, picked),
        owned,
    ) = drafts.get(accept.draft)?;
    if unfinished((points, forced, allotted), (abilities, training, usable)).is_some() {
        return Ok(());
    }
    let Some(picks) = picks_left(granted, calling, &picked.0, &content) else { return Ok(()) };
    let (mut was, mut gained, mut base, mut trained, mut left) = members.get_mut(member)?;
    was.set_if_neq(calling.clone());
    gained.set_if_neq(*experience);
    base.set_if_neq(*abilities);
    trained.set_if_neq(training.clone());
    left.set_if_neq(picks);
    // What was picked, the member knows.
    let owned = owned.into_iter().flat_map(|owned| owned.iter());
    for castable in owned.filter(|&castable| picked_spells.contains(castable)) {
        commands.entity(castable).remove::<PickedSpell>().insert(CastableOf(member));
    }
    commands.trigger(crate::game::inventory::omnigun::Advanced { member });
    commands.entity(accept.draft).despawn();
    Ok(())
}

/// A draft whose member changed after it was made goes: what it was made
/// from is no longer what they are. What its own acceptance wrote is older
/// than any draft made after it.
#[allow(clippy::type_complexity)]
fn let_go_stale(
    drafts: Query<(Entity, &Becomes, Ref<Drafted>)>,
    members: Query<
        (
            Ref<Calling>,
            Ref<Abilities>,
            Ref<Training>,
            Option<Ref<Castables>>,
            Ref<Experience>,
            Ref<SpellPicks>,
        ),
        Without<Drafted>,
    >,
    ticks: bevy::ecs::system::SystemChangeTick,
    mut commands: Commands,
) {
    for (draft, &Becomes(member), made) in &drafts {
        let Ok((calling, abilities, training, learned, experience, picks)) = members.get(member)
        else {
            continue;
        };
        let changed = [
            calling.last_changed(),
            abilities.last_changed(),
            training.last_changed(),
            experience.last_changed(),
            picks.last_changed(),
        ];
        let learned = learned.map(|learned| learned.last_changed());
        if changed
            .iter()
            .chain(learned.iter())
            .any(|tick| tick.is_newer_than(made.added(), ticks.this_run()))
        {
            commands.entity(draft).despawn();
        }
    }
}

/// The session a character being made belongs to; one not chosen for the
/// party goes with it, and one who joins is no longer of it.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Making)]
pub struct MadeIn(pub Entity);

/// The characters being made in a session.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = MadeIn, linked_spawn)]
pub struct Making(Vec<Entity>);

/// One being made to start a game: their skills are seeded from their
/// attributes as those change, and they pick first spells (its hook).
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = newborn)]
pub struct Newborn;

fn newborn(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    world.commands().entity(context.entity).observe(pick_first).observe(conceive);
}

/// The attribute points a race, gender and profession leave to spend: 60
/// less what raising the race's attributes to the profession's minimums
/// costs (retail `ApplyProfessionMinimumAttributes`); none for a race with
/// no attributes of its own, a male Valkyrie, or minimums past 60.
pub fn budget(
    race: super::Race,
    gender: super::Gender,
    profession: Profession,
) -> Result<u32, &'static str> {
    let base = race.row().attributes.ok_or("Choose a playable race.")?;
    if profession.row().only_gender.is_some_and(|only| only != gender) {
        return Err("Valkyries must be female.");
    }
    let required: u32 =
        profession.row().minimums.iter().map(|(a, &minimum)| minimum - minimum.min(base[a])).sum();
    if required > 60 {
        return Err("This race cannot meet the profession's requirements.");
    }
    Ok(60 - required)
}

/// What a new character's skills start at: each of the profession's skills
/// and its bonus skill at its base, the average of its two attributes, times
/// 60 / (1 + the number of profession skills) / 100 (retail
/// `RebuildSkillAllocations`).
pub fn seeded(profession: Profession, abilities: &Abilities) -> Training {
    let row = profession.row();
    let mut training = Training::default();
    let share = 60 / (1 + row.skills.len() as u32);
    for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
        training.0[skill] = super::skill_base(abilities, skill) * share / 100;
    }
    training
}

/// A character to be made is given a race, a gender and a profession, at
/// first or anew: what was spent comes back; they start at the race's
/// attributes, raised to the profession's minimums out of the 60; with the
/// recipe's portrait, personality and voice, and the profession's first
/// picks.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Conceive {
    #[event_target]
    pub draft: Entity,
    pub race: super::Race,
    pub gender: super::Gender,
    pub profession: Profession,
}

fn conceive(
    conceive: On<Conceive>,
    mut drafts: Query<&mut Identity>,
    mut commands: Commands,
) -> Result {
    let Conceive { draft, race, gender, profession } = *conceive;
    let Ok(points) = budget(race, gender, profession) else { return Ok(()) };
    let base = race.row().attributes.ok_or("a playable race has attributes")?;
    let minimums = &profession.row().minimums;
    let abilities = Abilities(EnumMap::from_fn(|a| base[a].max(minimums[a])));
    let (portrait, personality, voice) =
        super::creation::identity::defaults(race, gender, profession);
    let mut identity = drafts.get_mut(draft)?;
    identity.portrait = portrait.into();
    identity.personality = personality;
    identity.voice = voice;
    let mut picks = SpellPicks::default();
    picks.grant(profession, magic::starting_picks(profession));
    commands.entity(draft).insert((
        super::Birth { race, gender },
        // What the race is of itself, which its figures read: resistances,
        // carrying and armour.
        crate::game::actors::Nature::of_race(race),
        crate::game::actors::armour::born(race),
        Calling::begun(profession),
        seeded(profession, &abilities),
        abilities,
        picks,
        Points { attributes: points, skills: 15 },
        Forced::default(),
        Allotted::default(),
        Picked::default(),
    ));
    Ok(())
}

/// A character to be made, before a race, gender and profession are chosen:
/// a Human Fighter, as the original's first page offers.
pub fn newborn_bundle(name: &str) -> impl Bundle {
    let identity = Identity { name: name.to_owned(), full_name: name.to_owned(), ..default() };
    (Drafted, identity, Vitals::full(1, 1), Experience::default())
}

/// A new character's skills follow their attributes: seeded from them, and
/// what was put in on top.
pub fn reseed(
    mut drafts: Query<
        (&Calling, &Abilities, &Allotted, &mut Training),
        (With<Newborn>, Or<(Changed<Abilities>, Changed<Allotted>, Changed<Calling>)>),
    >,
) {
    for (calling, abilities, allotted, mut training) in &mut drafts {
        let mut now = seeded(calling.profession, abilities);
        for (skill, &put) in &allotted.skills {
            now.0[skill] += put;
        }
        training.set_if_neq(now);
    }
}

/// A new character picks one of the first-level spells of their books
/// (spells the game has, below the original's own), while picks are left;
/// one picked is given back; with all picks spent, the earliest picked but
/// the last gives way (retail `SelectSpell`). What they know follows.
#[allow(clippy::type_complexity)]
fn pick_first(
    pick: On<Pick>,
    mut drafts: Query<(&Calling, &SpellPicks, &mut Picked)>,
    content: Res<Catalogs>,
) -> Result {
    let (calling, granted, mut picked) = drafts.get_mut(pick.draft)?;
    let id = pick.spell;
    if let Some(at) = picked.0.iter().position(|&spell| spell == id) {
        picked.0.remove(at);
    } else {
        let learnable =
            content.spells.get(id).is_some_and(|spell| may_pick_first(calling, granted, spell));
        if !learnable {
            return Ok(());
        }
        let mut more = picked.0.clone();
        more.push(id);
        if picks_left(granted, calling, &more, &content).is_none() {
            let last = picked.0.last().copied();
            let Some(replaced) =
                picked.0.iter().copied().filter(|&s| Some(s) != last).min_by_key(|s| s.0)
            else {
                return Ok(());
            };
            more.retain(|&s| s != replaced);
        }
        picked.0 = more;
    }
    Ok(())
}

/// Whether a new character may pick a spell first: a first-level spell of
/// a realm, of one of their books, that the game has below the original's
/// own, if they pick any at all.
pub fn may_pick_first(
    calling: &Calling,
    granted: &SpellPicks,
    spell: &crate::content::spells::SpellDefinition,
) -> bool {
    let books =
        if granted.total() == 0 { EnumSet::empty() } else { calling.profession.row().books };
    spell.id.0 > 0
        && spell.id.0 < 0x72
        && spell.realm.is_some()
        && spell.level == 1
        && !(spell.books & books).is_empty()
}

/// The longest full name and nickname, in characters.
pub const NAME_LIMIT: usize = 39;
pub const NICKNAME_LIMIT: usize = 9;

/// Whether a nickname is another's already: nicknames tell characters
/// apart (retail `ValidateName`), compared trimmed.
pub fn nickname_taken<'a>(name: &str, others: impl IntoIterator<Item = &'a Identity>) -> bool {
    others.into_iter().any(|other| other.name.trim() == name.trim())
}

/// Why a new character may not join yet: a name, a nickname, a portrait, or
/// points left to spend. Picks may be kept for later.
pub fn unready(
    identity: &Identity,
    (points, forced, allotted): (&Points, &Forced, &Allotted),
    (abilities, training, usable): (&Abilities, &Training, &UsableSkills),
) -> Option<&'static str> {
    let bad = |name: &str| name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c));
    if identity.full_name.trim().is_empty()
        || identity.full_name.chars().count() > NAME_LIMIT
        || bad(&identity.full_name)
    {
        return Some("Enter a valid full name of 1–39 characters.");
    }
    if identity.name.trim().is_empty() {
        return Some("Enter a nickname.");
    }
    if identity.name.chars().count() > NICKNAME_LIMIT || bad(&identity.name) {
        return Some("Enter a valid nickname of at most nine characters.");
    }
    if identity.portrait.is_empty() || !identity.portrait.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        return Some("Choose a portrait.");
    }
    unfinished((points, forced, allotted), (abilities, training, usable))
}

/// A new character is named, or given a portrait, personality or voice.
#[derive(EntityEvent, Clone, Debug)]
pub struct Introduce {
    #[event_target]
    pub draft: Entity,
    pub identity: Identity,
}

fn introduce(introduce: On<Introduce>, mut drafts: Query<&mut Identity, With<Drafted>>) -> Result {
    drafts.get_mut(introduce.draft)?.set_if_neq(introduce.identity.clone());
    Ok(())
}

/// A new character joins the party, complete (retail
/// `FinalizeCreatedCharacter`, `AddCharacterStartingEquipment`): what was
/// spent is theirs, their goal is their profession's weight, their health,
/// stamina and points are full, and they carry their starting equipment.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Join {
    #[event_target]
    pub draft: Entity,
    pub party: Entity,
}

fn join(
    join: On<Join>,
    drafts: Query<
        (&Identity, &super::Birth, &Calling, &Training, &SpellPicks, &Picked, Option<&Castables>),
        With<Drafted>,
    >,
    content: Res<Catalogs>,
    mut rng: ResMut<crate::game::rng::GameRng>,
    mut commands: Commands,
) -> Result {
    let Join { draft, party } = *join;
    let (identity, birth, calling, training, granted, picked, owned) = drafts.get(draft)?;
    // What they picked, they know.
    for castable in owned.into_iter().flat_map(|owned| owned.iter()) {
        commands.entity(castable).remove::<PickedSpell>();
    }
    let picks = picks_left(granted, calling, &picked.0, &content)
        .ok_or("a joined character's picks hold")?;
    let skills = Skills::affected(training, calling, &super::Conditions::default());
    let starting =
        crate::game::party::roster::starting_items(birth.race, calling.profession, &skills);
    commands
        .entity(draft)
        .remove::<(Drafted, Newborn, MadeIn, Points, Forced, Allotted, Picked)>()
        .insert((
            Name::new(identity.name.clone()),
            picks,
            crate::game::party::roster::PartyMember(party),
        ));
    crate::game::party::roster::outfit(&mut commands, draft, &starting, &content, &mut rng);
    Ok(())
}

pub fn plugin(app: &mut App) {
    app.add_systems(Update, let_go_stale)
        .add_observer(introduce)
        .add_observer(join)
        .add_observer(allot)
        .add_observer(advance);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Realm;
    use crate::content::characters::{Gender, Race};
    use crate::content::spells::{SpellCatalog, SpellDefinition, Spellbook};
    use crate::game::characters::born;

    /// Whether an actor knows a spell: has learned it as a castable.
    fn knows(world: &mut World, actor: Entity, id: SpellId) -> bool {
        let owned: Vec<Entity> =
            world.get::<Castables>(actor).map_or(vec![], |c| c.iter().collect());
        owned.into_iter().any(|c| world.get::<CastableSpell>(c).is_some_and(|s| s.0.id == id))
    }

    /// Three first-level Divinity spells, as the game's catalogue has them.
    fn divine_firsts() -> Catalogs {
        let first = |id: u8| SpellDefinition {
            id: SpellId(id),
            realm: Some(Realm::Divine),
            level: 1,
            cost: 1,
            books: Spellbook::Divinity.into(),
            ..default()
        };
        let mut spells: Vec<SpellDefinition> =
            (0..=20).map(|id| SpellDefinition { id: SpellId(id), ..default() }).collect();
        for id in [6, 13, 16] {
            spells[id as usize] = first(id);
        }
        Catalogs { spells: SpellCatalog::new(spells), ..default() }
    }

    /// A new Priest spends every attribute and skill point within their
    /// steps and picks their two first spells; a third pick gives way to the
    /// earliest but the last; with a name, a nickname and a portrait they
    /// are ready to join.
    #[test]
    fn a_new_priest_spends_every_point_and_picks_two_first_spells() {
        let mut app = App::new();
        app.add_plugins(crate::game::actors::plugin)
            .init_resource::<crate::game::rng::GameRng>()
            .insert_resource(divine_firsts());
        let world = app.world_mut();
        let draft = world.spawn((newborn_bundle("Pia"), Newborn)).id();
        let (race, gender, profession) = (Race::Human, Gender::Female, Profession::Priest);
        world.trigger(Conceive { draft, race, gender, profession });
        world.flush();
        crate::game::actors::settle(world);
        let points = *world.get::<Points>(draft).unwrap();
        assert_eq!(points.skills, 15);
        for attribute in EnumSet::<Attribute>::all() {
            for _ in 0..30 {
                world.trigger(Allot { draft, what: Allotment::Attribute(attribute), up: true });
            }
        }
        world.flush();
        crate::game::actors::settle(world);
        let (forced, allotted) =
            (world.get::<Forced>(draft).unwrap(), world.get::<Allotted>(draft).unwrap());
        assert_eq!(attributes_left(&points, forced, allotted), 0, "the pool is spent");
        assert!(allotted.attributes.values().all(|&put| put <= step(points.attributes)));
        for spell in [6, 13, 16] {
            world.trigger(Pick { draft, spell: SpellId(spell) });
        }
        world.flush();
        let picked = &world.get::<Picked>(draft).unwrap().0;
        assert_eq!(picked, &[SpellId(13), SpellId(16)], "the third gives way to the first");
        crate::game::actors::settle(world);
        let usable = world.get::<UsableSkills>(draft).unwrap().0;
        for skill in usable.iter() {
            for _ in 0..6 {
                world.trigger(Allot { draft, what: Allotment::Skill(skill), up: true });
            }
        }
        world.flush();
        crate::game::actors::settle(world);
        let identity = Identity {
            name: "Pia".into(),
            full_name: "Pia of Arnika".into(),
            portrait: "humf1".into(),
            ..default()
        };
        world.trigger(Introduce { draft, identity });
        world.flush();
        let mut ready = world.query::<(
            &Identity,
            (&Points, &Forced, &Allotted),
            (&Abilities, &Training, &UsableSkills),
        )>();
        let (identity, spent, sheet) = ready.get(world, draft).unwrap();
        assert_eq!(unready(identity, spent, sheet), None);
        assert!(knows(world, draft, SpellId(16)));
    }

    /// A Priest whose experience has reached the goal gains a level: the
    /// draft made of them lasts while frames pass, takes the level's 6
    /// attribute and 9 skill points, at most 3 apiece, and a spell, and
    /// accepting it writes the level and the spell onto them; the draft is
    /// gone.
    #[test]
    fn a_member_gains_a_level_through_a_draft_gone_once_accepted() {
        let mut app = App::new();
        app.add_plugins(crate::game::actors::plugin)
            .init_resource::<crate::game::rng::GameRng>()
            .insert_resource(divine_firsts());
        let world = app.world_mut();
        let member = world.spawn(born("A", Race::Human, Gender::Male, Profession::Priest)).id();
        crate::game::magic::castable::test_learn(world, member, &[SpellId(16)]);
        crate::game::party::roster::test_party(world, &[member]);
        let goal = world.get::<Calling>(member).unwrap().experience_goal(&Experience::default());
        world.get_mut::<Experience>(member).unwrap().earned = goal;
        crate::game::actors::settle(world);
        let strength_before = world.get::<Abilities>(member).unwrap().0[Attribute::Strength];

        world.trigger(Advance { member });
        world.flush();
        // Frames pass while the player chooses: nothing lets the draft go.
        for _ in 0..3 {
            app.update();
        }
        let world = app.world_mut();
        let mut drafts = world.query::<(Entity, &Becomes)>();
        let (draft, _) = drafts.single(world).expect("one draft of the member");
        assert_eq!(world.get::<Points>(draft), Some(&Points { attributes: 6, skills: 9 }));

        for _ in 0..4 {
            world.trigger(Allot {
                draft,
                what: Allotment::Attribute(Attribute::Strength),
                up: true,
            });
        }
        for attribute in [Attribute::Vitality, Attribute::Dexterity] {
            for _ in 0..3 {
                world.trigger(Allot { draft, what: Allotment::Attribute(attribute), up: true });
            }
        }
        world.flush();
        let allotted = world.get::<Allotted>(draft).unwrap();
        assert_eq!(allotted.attributes[Attribute::Strength], 3, "no more than 3 an attribute");
        crate::game::actors::settle(world);
        let usable = world.get::<UsableSkills>(draft).unwrap().0;
        for skill in usable.iter().take(3) {
            for _ in 0..3 {
                world.trigger(Allot { draft, what: Allotment::Skill(skill), up: true });
            }
        }
        world.trigger(Pick { draft, spell: SpellId(6) });
        world.flush();
        crate::game::actors::settle(world);

        world.trigger(Accept { draft });
        world.flush();
        assert!(world.get_entity(draft).is_err(), "the draft is gone");
        assert_eq!(world.get::<Calling>(member).unwrap().level(), 2);
        let strength = world.get::<Abilities>(member).unwrap().0[Attribute::Strength];
        assert_eq!(strength, strength_before + 3);
        assert!(knows(world, member, SpellId(6)), "the spell picked");
        assert!(knows(world, member, SpellId(16)), "and the spell known before");
    }
}
