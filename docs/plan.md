# Rewrite plan

`game/` is being rewritten from the game's rules up, by the rules in
[design.md](design.md). Earlier slices made items, levels, tables, the round,
conversation, the story and devices into entities, events and data; what is
left of the port is how rules read actors and content. This page says why,
and the steps that remove it. What was done before is in git.

## What every gameplay change must satisfy

1. Behaviour is a system or an observer. A plain function takes numbers and
   plain values, never a component handed to it by a caller.
2. Kind is which components an entity has. No system takes an optional
   component and branches on it; what differs is data on the entity.
3. Derived state is a component, written by one system when its inputs
   change. Nothing recomputes it on demand; screens only query.
4. Everything rules apply to is an entity. No drafts, previews or templates
   as values that need the rules as callable functions.
5. A query is the interface. No `QueryData` or `SystemParam` that gathers a
   dozen components into an object to pass around.
6. No raw encodings in `game`; content is decoded into the game's terms
   where it is read.
7. No invented concepts. A component is named for a thing the game has.

A rule is first written in words from the original, in its page under
[rules/](rules/); then its components; then its systems. The existing Rust is
not a source.

## Why the bundles exist

Rules are still procedures handed a whole actor: `actors::Body` (87 uses),
`combat::attack::Fighter` (38), `characters::Trained`/`Schooling` (90),
`hits::reach::Field` (48) and the monster's `Sight`. Each bundle feeds one of
four causes, and removing a bundle without its cause only moves the
procedure.

1. **Derived facts are worked out on read.** The side an actor is on now
   (`side.now(conditions)`, 20 places), fatigue, agility, the eyes: each needs
   several components, so every rule that reads one needs the actor whole.
2. **Characters on paper are values.** `Character`, the level-up draft and
   the `Trained` view exist so that rules can run on a character that does
   not exist; that is why rules are functions of a view.
3. **What a thing is is looked up by id.** 45 files in `game` read
   `Catalogs` as rules run (38 spell lookups alone).
4. **Rules decide by scanning everyone.** Whom a cast reaches and who
   notices whom loop over every actor, so they need the world gathered into
   `Field` and `Sight`.

## Steps

Each step removes one cause and then the bundles that rested on it. Each
ends with clippy and the retail tests; CI runs the rest. Each reports, before
and after: uses of `Body`, `Fighter`, `Field`, `Sight`, `Trained`; readers
of `Catalogs`; `QueryData`/`SystemParam` in `game` (43 at the start).

1. **Derived facts are figures.** Done: where an actor stands now
   (`hostility::Standing`, its side turned by Turncoat and the band it keeps
   to), its `Fatigue`, its `Agility` and its `Eyes` are components, each
   written by one system as part of settling bodies; `Body::stance`,
   `standing`, `attack::fatigue` and every caller of `Eyes::of` are gone. A
   figure follows its sources by the next settling, so a Turncoat laid by a
   deed's row is felt from the next moment, not by the deed's later rows.
2. **An attack is entities, not a procedure.** Done. No rule function takes a
   handful of numbers it was handed, no struct (`Attack`, `Fighter`, `Body`)
   carries copied values to a rule, and no observer is a long procedure.
   - **An arm is an entity** related to its bearer: a member's hand that
     strikes (what it holds by relationship, a fist when empty), a monster's
     natural attack. Each fact of it is a component written by one small
     system from what it comes from: its rating, reach, modes, dice and
     punch/kick dice, raise in percent, effects, the kinds it slays, attacks
     and swings a round, initiative, what it loads, its critical skill, the
     skills it trains, its weight, whether it tires, is modern, tires on
     penetration, is a sword, ambushes, its word, where it lands on a member,
     its sound. A monster's arms are made with it from its species; a
     member's follow what they hold and their training. `Attacks.arms` and
     `Hands`'s procedures go.
   - **The rest of `Attacks` splits** into one component per fact
     (initiative, `Pressing`, fighting blind, keen sight, backstab, knocking
     out, stride, haste); what only some actors have is present only on
     them.
   - **What catches an actor out** is its own components, each with a small
     system: `Unaware`, `Unseated`, `Block`, `Blinded` (written already).
   - **A swing is an entity**, spawned with its attacker, target, arm, mode
     and how it caught the target. Each fact of it is a component added by
     a small observer that reads only what that fact comes from: its `Aim`
     (the arm's rating and the mode's to-hit, the swing's surprise, the
     attacker's `Fatigue`, `Agility` and blindness, the target's armour,
     `Unaware` and `Agility`); `Reached`, `Missed` or `Astray`; a guard
     stepping in (`Pressing` against `Pressing`); a near miss `Blocked`;
     where it `LandsAt`; whether it gets `Through` (the armour at the place,
     `Against`, `MissileShield`, the arm's penetration); its `Damage`, which
     makes the blow's `Deed`; what it costs and uses up. The attack's tally
     and the next swing follow the swing being done.
   - **Choosing whom to strike** is likewise facts added to the attack
     (whom it looks for, whose eye misses, the ways it could strike, the one
     picked or closing in), each by its own small observer.
   `Fighter`, `Attack`, `Body`'s use by the attack, and every
   `chance_to_hit`/`penetration`/`blow`-style function go.

   *Done:* arms are entities (`actors/arm.rs`): a member's two hands, each
   fact a component with its own system (ported from `Hands`, which is
   gone), and a monster's natural attacks made with it; what a bearer's
   arms come to is `Wielding`, `Tempo`, `Furthest`, `GuardReach`,
   `Pressing` and `AimBonus`. An attack and each of its swings are
   entities (`combat/swings`), their facts added by small observers;
   `Fighter`, `Attack`, `Setting`, `NaturalArms`, `accept_round` (now the
   `Accepts` event and its observers) and the fighter-taking rules are
   gone. Choosing whom to strike is facts of the attack being chosen
   (`Sought`, `Ways`, the one picked, or closing in). `Attacks` is split:
   `Initiative` (with `TrainedInitiative`, nothing for one who never
   trained), `BlindFighting` and `KnockOut` only on those who have them,
   `Backstab`; keen sight, stride and haste are read from `Nature` and
   `Modifiers`. The arms' retail rules are pinned again against the hands.

3. **A cast is an entity, and `Field` goes.** As the attack: the `Cast`
   event makes a cast entity, with who casts, the spell's definition (read
   from content as the cast is made) and what was chosen (its source and
   aim); each fact of it is added by a small observer reading only what
   that fact comes from, in the order of [rules/casting.md](rules/casting.md):
   - whether it may be cast at all (`may`), or it is refused and gone;
   - what it is cast with (`CastWith`: a learned spell, an item, a
     monster's own), which says how it is paid for and how strong it is;
   - whom it reaches: one observer per kind of target (the caster, a
     friend, the party, a foe, a foe's group, every foe, a cone, a blast's
     spot, a utility spell's mark), each reading where those about stand,
     their side and their group. The aim is a component of its own, so a
     blind caster's aim gone astray is a new aim and whom it reaches follows;
   - its power; the roll (cast, fizzled, backfired); what a backfire turns
     it on;
   - what it costs (`Pays`, a use of an item) and trains;
   - its release: a deed with what follows it beside it, or the utility
     spell's own event, or a fizzle's report and an auto cast again.
   Who may be taken (the party, the fight's combatants and the rest of their
   groups, the targetable among them) is a figure, written once, not worked
   out by each rule. `Field`, `Aiming`, `Means`, `Circumstances` and `Marks`
   as bundles, and `BodyItem` in `may`, `cost`, `means`, `workings` and
   `landed`, go.
4. **A monster's choices are observers.** Done
   ([rules/monsters.md](rules/monsters.md), "How this is built"): a
   deliberation is an entity whose facts are added in the order of section 4;
   each spell weighed is a child entity with its aims; entering a fight weighs
   the same way. `Mind`, `Fray`, the candidate struct, `deliberate`'s
   procedure, `acts_on_party` and every monster rule taking a `BodyItem` are
   gone; `Sight` keeps who stands where, on which side, and who sees whom.
5. **Drafts are entities.** Level-up makes an actor for the member to be, as
   creation does, its figures written by the same systems; accepting writes
   its sources onto the member. `Character` as a struct, `Trained`,
   `Schooling`, `of_sheet` and the screens that work from paper go.
6. **Content is components where things are made**, area by area: what a
   monster or an item casts carries its spell's definition; an item its name,
   value, class and stack limit; a monster its faction, stride, senses and
   treasure; a person their names, merchant, voice, faction and what they
   know, so that `npcs::access::NpcActors` keeps only where each can be met.
   `Catalogs` is read only where things are made.
7. **Tests drive the real plugins.** `combat/tests.rs` and
   `integration/level_lifecycle.rs` move onto the game's plugins; tests of
   removed types are deleted, not rebuilt.

## In progress: singletons, events and lookups

Taken over mid-slice. The tree compiles and clippy is clean; **no test has
been run since `8709303`** — run the unit suite, then the retail tests
(`RIZZARDRY_RETAIL=../wizardry-prepared cargo nextest run --run-ignored
only`), before going further.

What this slice settled, as rules for the rest:

- A sequence is events, not components added: the one that starts it queues
  its moments in order; observers of one event are independent of one
  another (swings, the monster's deliberation).
- An observer that hears events meant for others is attached to the entity
  it is about instead: by the spawn site (`.observe` before the deed is
  inserted, since a deed lands as it is made) or by an `on_add` hook on the
  marker (`Magical`, `Leech`, UI rows). Its lookups are then `?`.
- A lookup that must hold is `?` (observers and systems return `Result`) or
  `expect`; what may be missing is `Option`/`Has` in the query and an
  explicit branch, never a query that fails to filter.
- `Single` and `Option<Single>` skip silently on none *and* on several, so an
  invariant singleton is a `Resource` on its entity (Bevy 0.20 resources are
  components): `Battle`, `Party`, `PartyCamera`, and `InPlay` on the played
  level's root (moved by `activate_level`). A fight ends by
  `remove_resource::<Battle>()`; `Battle`'s `on_remove` hook takes its
  entity with it.
- "Only while X" / "never while X" is a run condition on the registration
  (`resource_exists::<Battle>`, `not(...)`), or `If<Res<X>>` when the system
  reads X; `Option<Res<X>>` only where both cases do something.
- A refactor redesigns its readers too; "readers stay as they are" is not a
  rule.

Left to do:

1. **No party in events or places.** Drop `party` from `Give`, `Swap`,
   `Pass`, `Throw`, `Merge`, `Identify` and `Advanced` (39 senders) and from
   `Place::Hand`/`Place::Pool` (32 sites); their observers read
   `Single<Entity, With<Party>>`. `Throw`'s `level` is the level in play.
   `Items` keeps taking the holder: a member holds as the party does.
2. **Readers of the new resources.** Queries that loop over or `single()`
   the party (`actors/time.rs` `parties`, `actors/place.rs`,
   `actors/modifiers.rs`, `inventory/omnigun.rs`, `npcs/population.rs`,
   `npcs/trade.rs`, presentation's `Query<…, With<PartyBody>>`) become
   `Single`; non-`Single` readers of `With<Level>` that mean the level in play
   become `With<InPlay>`. Review each remaining `Option<Res<Battle>>`
   (`request_camp`, `pass`, `accept_magic`, `overhear`, `awaited`,
   `write_off_wiped_out`, `confront`, `answer_travel_questions`, `travel`,
   `toggle_search`, `search_pulse`, the bank's `press`, `choose_row`): most
   act in both cases, but check.
3. **The rest of the lookup sweep.** About 80 `if let Ok(..) = q.get(..)` and
   250 `.get(..).ok()` / `.is_ok_and` / `.map_or` remain; the same treatment
   as the `let … else` sweep (query always matches, `?`, explicit branch, or
   an observer on the entity).
4. **Tuple patterns on bools** that predate this slice: `slot.rs:115`,
   `special.rs:91`.

Risks to look for when the tests run: a `?`/`expect` that panics where the
old code returned quietly (`take_up`/`act` for a monster without arms,
`rest_the_idle` for a combatant without `Mending`, hostility's `made` for an
`Allegiance` without `Loyalty`); a `Res<Battle>` beside a query over `Battle`
data, which Bevy rejects at startup (B0002), not at compile time; a test
that spawns two fights or parties in one world, of which the second is now
dropped with a warning.

## Done, and not to be looked for again

- Effects land as events, one observer for each thing a row changes; blows
  and Razor Cloak's answer are deeds; a cast pays through `Pays`; nothing
  writes through a bundle.
- A turn is a chain (`Moment`, `Deliberates`, `Decided`); the `Mind` bundle
  is gone.
- Armour is its parts, summed ([rules/arms.md](rules/arms.md)); the most
  spell points is a figure, and a learned spell carries its realm and cost
  ([rules/casting.md](rules/casting.md)); `Mastery` is written where an
  actor is made; death, cheated or not, is settled in one place (`fall`).
- Devices, encounters, items, levels, tables, conversation and the story are
  their rule pages' "How this is built".

Checked and **not** a problem: modules reading another domain's state;
retail files as assets beside compiled-in tables; `Items` as the "what is
held" accessor and `npcs::story::Readings` as the one reader of the story's
facts; a level reading its own definition; `#[cfg(test)]` helpers beside
their code; large files as such.
