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

## What is left of the port

Rules still read whole actors and content through bundles: `actors::Body`
(about 60 uses), `characters::Trained`/`Schooling` (about 40),
`hits::reach::Field` (about 13); 44 files in `game` read `Catalogs` as rules
run; `game` has 40 `QueryData`/`SystemParam`. Each rests on one of three
causes, and removing a bundle without its cause only moves the procedure:

1. **Characters on paper are values.** `Character`, the level-up draft and
   the `Trained` view exist so that rules can run on a character that does
   not exist.
2. **What a thing is is looked up by id** while rules run, rather than
   carried by the thing.
3. **Whom a cast reaches is worked out by scanning everyone**, so it needs
   the world gathered into `Field`.

## Steps

Each step removes one cause and then the bundles that rested on it, and
reports before and after: uses of `Body`, `Field`, `Trained`; readers of
`Catalogs`; `QueryData`/`SystemParam` in `game`.

Done: derived facts are figures (step 1); an attack is entities (step 2,
[rules/attack.md](rules/attack.md), [rules/arms.md](rules/arms.md)); a
monster's choices are observers (step 4,
[rules/monsters.md](rules/monsters.md)).

3. **A cast is an entity, and `Field` goes** (cause 3). The model is in
   [rules/casting.md](rules/casting.md), "How this is built".
5. **Drafts are entities** (cause 1). Level-up makes an actor for the member
   to be, as creation does, its figures written by the same systems;
   accepting writes its sources onto the member. `Character` as a struct,
   `Trained`, `Schooling`, `of_sheet` and the screens that work from paper
   go.
6. **Content is components where things are made** (cause 2), area by area:
   what a monster or an item casts carries its spell's definition; an item
   its name, value, class and stack limit; a monster its faction, stride,
   senses and treasure; a person their names, merchant, voice, faction and
   what they know, so that `npcs::access::NpcActors` keeps only where each
   can be met. `Catalogs` is read only where things are made.
7. **Tests drive the real plugins.** `combat/tests.rs` and
   `integration/level_lifecycle.rs` move onto the game's plugins; tests of
   removed types are deleted, not rebuilt.

## In progress: step 3, a cast is an entity

The last slice (a moment in a fight, from decision to deed; weapon sets;
setting aside) is done and pushed (`516e8e1`): unit suite 175/175, retail
30/30. What it built is under "Done" below and in the rule pages' "How this
is built". Step 3 is next; its model is written in
[rules/casting.md](rules/casting.md) ("How this is built") and **waits for
the user's approval before any code**.

Rules settled so far, which hold for what comes next:

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
- An action, an attack and a deliberation are made by whoever decides, who
  then triggers their sequence (`Acts`, `Seeks`, `Begun`); what follows an
  outcome (`Found`, `Nobody`) is attached to the entity by its maker. What
  differs by kind (member or monster) is an observer attached where the kind
  is made (`Order`'s and `Temper`'s observers in `combat::turn`), never a
  branch on an optional component.
- Being set aside is a state of a thing: a level left, or a companion out of
  the party, is only its root `Disabled`; each relationship whose held
  things go with their holder says so where it is declared
  (`world::aside::follows`). A new relationship does not follow unless it
  says so. Momentary and presentation relationships never do.
- Retail is read before a rule is written or corrected, and the claim cited
  (file and line). Twice this slice a rule written from memory was wrong
  (berserk's persistence, insanity's reach check).

### Next, in order

1. **Check `slot::fill`.** It despawns a spell slot from an observer of
   `Add<Casts>`. If `SlotOf` is spawned in the same bundle, the monster's
   `Slots` keeps a dead entry (the bug `Acting` had), and `look_to_open`'s
   `let Ok(..) else { continue }` hides it. Make the maker decide, or check
   before spawning.
2. **Step 3** once the user approves its model in `rules/casting.md`;
   `engage::Fighters` (the old `fighters_of` moved into an accessor) goes
   with `Body`.
3. Steps 5 to 7 as above.

### Open questions

- What a member's `W8_ACTION_RUN` (fear's exception, `Combat.cpp:2965`) is
  in this game; today only a monster's flight counts as a run.
- Whether disabling a level takes it out of physics and navigation, or
  their own handles must be removed and restored too
  ([rules/levels.md](rules/levels.md)).

### Left, low value

- The lookup sweep's remainder (about 65 `if let Ok`, 175 `.ok()`-style):
  optional facts and presentation, form rather than behaviour.
- Watch in play for a `?`/`expect` that panics where the old code returned
  quietly: `take_up` for a monster without arms, `rest_the_idle` for a
  combatant without `Mending`, hostility's `made` for an `Allegiance`
  without `Loyalty`.

### How the work goes

- A model goes into its rule page first and is shown to the user before
  coding.
- "Consult Fable" means the advisor tool. Consult it before a design and
  whenever a change is only moving code from one place to another.
- Clippy while working; the unit and retail suites once, before a commit;
  say in the commit what ran. Never build with `--features bevy/debug`
  (it rebuilds Bevy); probe with temporary `eprintln!` instead.
- Keep what `cargo fmt` produces. No `pub(crate)`: an item is `pub` or
  private.
- Tests state rules by outcome; a test that builds an entity from its
  internal components is deleted, not patched.

## Done, and not to be looked for again

- Effects land as events, one observer for each thing a row changes; blows
  and Razor Cloak's answer are deeds; a cast pays through `Pays`; nothing
  writes through a bundle.
- Derived facts are figures: where an actor stands (`Standing`), its
  `Fatigue`, `Agility` and `Eyes`, each written by one system as bodies
  settle.
- An attack is entities: arms (`actors/arm.rs`), the attack being chosen
  and each swing (`combat/swings`), their facts added by small observers.
- Armour is its parts, summed ([rules/arms.md](rules/arms.md)); the most
  spell points is a figure, and a learned spell carries its realm and cost
  ([rules/casting.md](rules/casting.md)); `Mastery` is written where an
  actor is made; death, cheated or not, is settled in one place (`fall`).
- Devices, encounters, items, levels, tables, conversation and the story are
  their rule pages' "How this is built".
- A moment in a fight is events and entities
  ([rules/monsters.md](rules/monsters.md), section 8 and "How this is
  built"): an action entity whose kind is its component, weighed by its
  actor's ailments in retail's order; `Done` and `Shown` end a moment;
  `Turn` is its facts and the round's markers; `Sight` is gone for
  `Eyesight` and the `PartyHides` figure; moving, charging and a member's
  breath are figures. `Plans`, `interrupt`, `friend_in_reach`, `act`,
  `Decision` and `Decided` are gone.
- Weapon sets swap by one rule ([rules/items.md](rules/items.md)): the
  player's (W, Shift+W), a Turncoat member's and the fight's own (the
  "swap weapons when needed" option), with retail's curses, notices and
  lost moments.
- Setting aside follows declared relationships
  ([rules/levels.md](rules/levels.md)); `Belongings` and its `unsafe` are
  gone.

Checked and **not** a problem: modules reading another domain's state;
retail files as assets beside compiled-in tables; `Items` as the "what is
held" accessor and `npcs::story::Readings` as the one reader of the story's
facts; a level reading its own definition; `#[cfg(test)]` helpers beside
their code; large files as such.
