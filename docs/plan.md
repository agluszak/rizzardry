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

## In progress: a moment in a fight, from decision to deed

The turn is still a procedure: `take_turn` mixes four things, `act` copies
components into `Plans` and calls `interrupt` and `friend_in_reach`, the
moment is ended by hand (`turn.end()` 13 times, the pause set 7), and
`Sight` hands every combat rule the world whole (41 calls of its getters).
This slice makes a moment events and entities from end to end, then goes on
to step 3. The model for parts 1 and 2 is written into
[rules/monsters.md](rules/monsters.md) ("How this is built") before any code.

Counted before and after: `Sight` and its getters' callers, `Plans`,
`interrupt`, `friend_in_reach`, `breath::available`, `closes_in`,
`turn.end()` and pause sites, `Field` uses.

Rules settled by the last slice (singletons, events and lookups), which hold
for this one:

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
- A resource's entity is Bevy's until `IsResource` leaves it: removing only
  the resource leaves the entity registered, and Bevy then refuses the
  resource anywhere else (a warning) or panics on a command when the entity
  is despawned. Moving or ending one removes `IsResource`, which takes the
  resource with it (`activate_level` for `InPlay`, `Battle`'s hook); nothing
  despawns a resource's entity directly.
- Nothing that can end an entity starts from `Add` (or `Insert`) of a
  bundle holding a relationship: Bevy adds to the relationship's list by a
  deferred command, so a chain that despawns the entity first leaves a dead
  entry. Whoever makes it triggers the sequence after spawning it
  (`Acts`).
- "Only while X" / "never while X" is a run condition on the registration
  (`resource_exists::<Battle>`, `not(...)`), or `If<Res<X>>` when the system
  reads X; `Option<Res<X>>` only where both cases do something.
- A refactor redesigns its readers too; "readers stay as they are" is not a
  rule.


### 1. An action is an entity, and its ailments are facts of it

Retail `GetConditionInterrupt` (`Combat.cpp:2940`) and its callers
(`:2176` a member's, `:2429` a monster's); the model is in
[rules/monsters.md](rules/monsters.md), section 8 and "How this is built".

- `Decided` gives way to an **action** entity made by whoever decided
  (`ActionOf`); what it is is which component it has (`Strikes`,
  `Retreats`, `Holds`, `Protects`, `Breathes`, `Casts`), and `AtFoes` is
  written by its maker. `Decision`, `Fights`, `Retreats` as a struct and
  `act`'s `match` go; each kind's doer is attached by its component's hook
  and hears `Unhindered`.
- Its ailments are one event and one observer each, in retail's order,
  each reading its own sources; what took the action is an event of its
  own (`Nauseated`, `BrokeFree`, …), each with its observer. `Plans`,
  `interrupt`, `Interrupt` as a value carried around (it stays only as the
  notice's number) and `friend_in_reach` go: insanity makes the strike at
  friends itself, whose search is retail's check (a monster needs sight and
  a clear line, `Combat Range.cpp:533`; a member looks among fellows).
- **Fixed against retail on the way:** a berserk actor whose later action
  an ailment takes strikes friends instead of losing it (`:2461`); a
  Turncoat action is told (`0x3a9`); a taken or berserk action does not
  provoke (`:2238`, `:2479`); fear's exception is the action being a run,
  not `Fleeing`.
- **New, and needing what is not here:** a Turncoat member who can strike
  nobody swaps weapon sets (retail `SwapWeaponSetSlots`). There is no
  weapon-set swap in the game yet; it is made as an event over the two
  sets' slots, and this rule uses it.
- To find out: what a member's `W8_ACTION_RUN` (fear's exception) is here.

### 2. A moment ends by events

Today `turn.end()` is written 13 times and the pause 7. They are two facts
that do not always go together: a strike holds the fight for its pause
with attacks left (`swings/mod.rs:342`), while holding back, a lost turn,
a refused breath, the party's move costing a member its moment
(`orders.rs:190`), waiting instead of striking (`swings/mod.rs:502`) and
standing for want of a goal (`:546`, `:570`) end the turn without one. So
the proposal is two events, `Done` and `Shown`, each one observer; one
event with a field would be a flag read by one observer. **Asked of the
user** before coding, since their outline said one event.

### 3. The rest of the turn

- **`Turn` splits** into one component per fact (`NextMoment`,
  `AttacksLeft`, `AcceptedWith`, `Lightning`) and a marker per thing the
  round did (`Moved`, `MadeSpecial`, `Waited`, `Berserk`), cleared as the
  next round is taken up. `take_up`'s branch on an optional
  `(&Order, &Traits, &Calling)` becomes an observer set where `Order` is
  made.
- **`take_turn` is a chain** (`Moment` → `Goes` → `Decides`), each one
  observer, because observers of one event cannot stop one another. The
  path for an actor with neither order nor temper goes: every monster gets
  its `Temper` where it is made (`population.rs:364`).
- **A breath to take is a figure**, `CanBreathe` (a marker like
  `SpecialReady`), written by one system from `Traits`, `Modifiers` (the
  breath laid, already summed) and `Vitals`. `breath::available` goes.
- **`Sight` goes**, each reader keeping its meaning: `side()` is
  `Side::own`; `toward()` is `hostility::between` on two `Standing`s, with
  the dead not counted and oneself a friend, written out where read; the
  gap is `Places`. How well the party hides (its lowest able level, and
  Chameleon) is a figure on the party, so seeing the party reads `Eyes`,
  places and that figure. `look`, `reinforce`, engage, pursuit, noise,
  round end, the deliberation and the aims read their own queries.
- **Moving in a fight is figures**, one fact each: how far it walks now
  (stride with haste or slowing) and how far at a charge, written by one
  system from `Nature`, `Modifiers` and `Conditions`; `Charges` (hurt to 95
  percent) by another from `Vitals`. Closing in reads both; a retreat never
  charges; the lure reads `Nature`'s stride, halved and unhasted.
  `closes_in`, `metres`, `MoveBudget` and `charges` go; `withdraws_to`,
  `arrived` and `needs_approach` stay arithmetic on numbers.

### Where it stands

Done, compiling and clippy clean; the unit suite passed before the last
two changes (`Sight`'s readers, the move figures), and has not been run
since:

- Parts 1 and 2: the action entity and its kinds (`combat/action.rs`), the
  ailments as ordered steps, each interruption its own observer, `Done` and
  `Shown`; `Plans`, `interrupt`, `friend_in_reach`, `act`, `Decision` and
  `Decided` are gone. The retail fixes above are in, but the Turncoat
  member's weapon-set swap.
- Part 3: `Turn` split into its facts and the round's markers; taking up a
  turn and deciding at a moment by observers set where `Order` and
  `Temper` are made; `CanBreathe`; `Sight` gone (who sees whom is
  `Eyesight`, the party's hiding the `PartyHides` figure); `Move` and
  `Charges` figures; ending the party's move is an event
  (`PartyMoveEnds`).
- Tests that built a turn from its components were deleted (seven in
  `combat/tests.rs`, the defending one in `armour.rs`, the arithmetic of
  `close.rs`).

Open:

- **A command on a despawned entity** in the retail scenario
  `the_party_fights_rests_and_finds_the_dead_still_dead`, not yet traced
  (rerun with `--features bevy/debug` to name it).
- The weapon-set swap is done (`inventory/weapon_sets.rs`,
  `combat/other_set.rs`): the player's (W, Shift+W), a Turncoat member's and
  the fight's own, one rule. Insanity and a Turncoat member ask the
  strike's own search whether they can strike someone; a member turned or
  berserk looks only among fellow members. Settled against retail: a
  cursed weapon in use blocks the swap until its curse is released, and
  moving it re-arms the curse (`PC Item.cpp:1293`, `:2149`, `:2788`); only
  the Turncoat swap takes up attacks anew, the fight's own strikes with
  those taken up, and either loses the rest of the round's moments
  (`alternate_hand`); a member mid-attack is refused (`0x1f7`). Not tested
  yet.
- A lead on the despawn failure, by reading: `charging` inserts `Charges`
  in the settle that kills a monster from full health; or `by_temper`'s
  `.observe` on a short-lived monster.
- **`engage::Fighters`** is the old `fighters_of` moved into an accessor;
  it goes with `Body` in step 3.

### 4. Then step 3, and steps 5 to 7

Step 3 (a cast is an entity, `Field` and `Body` go) reuses the monster's
slot, aim and takes-in model: a member's cast takes in whom it reaches as a
monster's aim does, so who may be taken is one rule. Its model goes into
[rules/casting.md](rules/casting.md) first. Steps 5 to 7 as above.

### Left from the last slice

- The lookup sweep's remainder (about 65 `if let Ok`, 175 `.ok()`-style)
  is optional facts and presentation: form, not behaviour, left.
- **A companion out of the party** is `Disabled` alone, while their hands,
  gear and spells stay in the world; they should be set aside whole, as a
  level's belongings are.
- Watch in play for a `?`/`expect` that panics where the old code returned
  quietly: `take_up`/`act` for a monster without arms, `rest_the_idle` for
  a combatant without `Mending`, hostility's `made` for an `Allegiance`
  without `Loyalty`.

### How the work goes

The model for 1 and 2 in `rules/monsters.md` first, shown before coding.
Clippy while working; the unit and retail suites once, before a commit.

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
