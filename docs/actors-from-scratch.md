# Actors, from scratch

Written 2026-11-12; updated 2026-11-16. Armour is built (below); nothing else
here is yet. It replaces an earlier plan that reshaped the existing code and
was rejected (`docs/redesign.md`, deleted; see git history).

## Why this exists

Commit `4954d57` split the character sheet and replaced `refit` with one
system per figure, but the rules stayed procedures: functions that are handed
a character and return a number, called from systems, from casting and from
the screens. Three attempts to fix that only renamed what the procedures were
handed (`&Character`, then a `Trained` view, then tuples). The last attempt is
in `git stash` ("tuple version of Trained removal (rejected)") and should stay
there.

## How the work is to be done

1. The existing Rust is not a source. For each thing, read the original
   (`../wizardry-8-decomp`), and write the rule down in plain words first.
2. Check facts before writing a type that depends on them.
3. Show the components before the systems, and one piece end to end before
   the rest.
4. No rename scripts and no working down the compiler's error list.

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
6. No raw encodings in `game`: no integer indices standing for enums, no
   `10 - x`, no byte widths. The content is decoded into the game's terms
   where it is read.
7. No invented concepts. A component is named for a thing the game has.

## Armour

Done. The rule is in [rules/arms.md](rules/arms.md), "Armour"; the code in
`src/game/actors/armour.rs`.

- `ArmourParts`: one small system per part, each reading what the part comes
  from. Defensive action reads the round's `Turn` (a member's Defend order,
  a monster's wait) and is cleared when the fight ends.
- `HitPlaces` and `Against`: set where the actor is made, from the race's
  table or the species, already decoded (`content::species::Species` carries
  armour, places, trunk and against with higher better). What a member wears
  rewrites the cover of their places.
- `Armour` `{ total, pierced, average }`: one system sums the parts. At a
  place is `Armour::at`, the total, pierced and the place's cover; no second
  list beside `HitPlaces`.
- What a blow gets past (unaware, on the move) is a plain function of the blow
  and the target in `combat/attack/aim.rs`, counted once against the aim and
  twice at a place; so is the block, heard as `CombatNews::Blocked`.

### Decided

- A monster's spells, tiredness and waiting are parts like a member's,
  counted once and capped out of formation; the original counts them twice
  at a place. One rule for every actor.
- Race armour is the race table's (only the Faerie has any), which is the
  original's Faerie trait.
- Fatigue's bands are the original's: under 50, 70, 85, 95 percent of stamina
  spent, for members and monsters alike.
- `VulnerableFromBehind` is inserted beside the species' bundle in `embody`:
  Bevy has no optional component in a bundle.

### Not here yet

- Protect and Pray orders for members, and with them their armour (+2 while
  protecting, 4 worse while praying).
- The armour panel of the character screen, listing `ArmourParts`
  (Reflextion only when not zero).
- What a monster's places are called (by its build, six of them), for the
  blow-by-blow lines, which are not ported.

## Spell points

Done. The rule is in [rules/casting.md](rules/casting.md), "Spell points".
`SpellPoints` holds the most each realm can hold beside what is spent; one
system (`figures::spell_points`) writes the most from `LearnedSpells`,
`Level`, Piety and `Skills`. A learned spell carries its realm and cost
from the moment it is learned. Readers (casting, draining, recovery, the
magic screen, creation) read the pool; `max_points` is called only by the
figure system.

## Not designed yet

Everything but armour and spell points: attributes, skills, the maxima,
resistances, attacks, mending, damage reduction, missile shield; what a
calling writes as sources; drafts as entities (creation's draft is an actor
for armour, attributes, resistances and spell points; the level-up draft is
not); casting and combat without sheet fields on `Body` and
`Means`. Each is to go through the same steps: the rule in words from the
original, then components, then systems.

What the armour left for the next figure (see [plan.md](plan.md), "Still in
the way"): `Body` and `Fighter` still gather every figure for reading,
armour's included, and should shrink to what each rule reads as figures are
rebuilt. Nothing writes through them any more.
Armour, mending, resistances and attacks now notice what is worn themselves
(`items::Redressed`); `Modifiers` is only the sum of what is laid.
