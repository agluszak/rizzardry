# Actors, from scratch

Written 2026-11-12; updated 2026-11-16. Armour is built (below, with what is
left of it); nothing else here is. It replaces the approach of
`docs/redesign.md`, which reshaped the existing code and was rejected.

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

The rule is in [rules/arms.md](rules/arms.md), "Armour". Built in
`src/game/actors/armour.rs` (commit `a3a7e96`): `ArmourParts`, `HitPlaces`,
`Against`, `Armour`, `VulnerableFromBehind`, one small system per part,
and one that sums them. The creator's draft is an actor, so its page reads
`Armour` like any other.

### The simplest shape, to finish

Components, as they should end up:

```rust
/// The parts, each written by what it comes from.
pub struct ArmourParts(pub EnumMap<ArmourPart, i32>);

/// The places of a body where a blow can land, set where the actor is made.
pub struct HitPlaces {
    pub places: Vec<HitPlace>,
    /// Where a blow lands that the chances do not place. Decoded with the
    /// body, never a literal in `game`.
    pub trunk: usize,
}

pub struct HitPlace {
    /// The part of a member's body it is: what is worn there covers it,
    /// and a monster's own chances for the parts apply to it.
    pub part: Option<BodyPart>,
    pub chance: u32,
    /// What covers it beyond the parts: a monster's from its record, a
    /// member's rewritten by `worn` when what is worn changes.
    pub cover: i32,
}

/// What it has against each way of attacking.
pub struct Against(pub EnumMap<AttackMode, i32>);

/// The parts summed.
pub struct Armour {
    /// Every part but vs Penetration, capped out of formation.
    pub total: i32,
    /// vs Penetration, where it counts (nothing for the dead and away).
    pub pierced: i32,
    /// For the portrait.
    pub average: i32,
}

/// The record flag: -2 caught unaware, and blocks by 10.
pub struct VulnerableFromBehind;
```

At a place is `total + pierced + cover`, added by the blow. No second list
parallel to `HitPlaces` that can disagree with it in length.

The blow's situation (unaware, praying, on the move) is one plain function of
the blow and the target, counted once against the aim and twice at a place.
It is not a part: it belongs to one blow, not to the actor.

### Left to do

1. **Drop `Armour::at`.** Store `pierced`; `penetration` and the devices'
   shot read `total + pierced + place.cover`.
2. **Decode monster armour where the species is read.** `armour::of_species`
   now does `10 - evasion`, `evasion - armour_by_location` and `trunk: 3`
   in `game`. The species content should carry armour (higher is better),
   the cover of each place, against-by-mode and which place is the trunk.
3. **`VulnerableFromBehind` with the rest of the species.** It is inserted
   in `embody`, not in `made_of`, so a monster made only by `made_of`
   (the combat tests) lacks it. Put it in `of_species`'s bundle.
4. **Defensive action for everyone.** Today only an `Ordered` holding
   `Defend` sets it. Protect counts too, and a monster that waits gets the
   same +2.
5. **Praying** (4 worse, while the party is not running) in the blow's
   situation.
6. **Blocking** (`BlockedForSpecialReason`): a blow that beats the armour by
   no more than the block does nothing. Write the block where it is known:
   five times the Shield part for a member, lost when caught unaware; 10 for
   a monster vulnerable from behind, whatever the surprise. The block plays
   the impact sound of the off hand's material (a monster's is material 2).
7. **The armour panel** on the character screen lists `ArmourParts`
   (Reflextion only when not zero).
8. **Same-settle check.** `worn` reads the Defensive action part to scale
   Shield skill. The shield test settles twice; find out whether a defend
   order and its shield bonus land in the same settle, and make them if not.

### Decided

- **Race armour** is the race table's (only the Faerie has any: 2), which is
  the trait's 2.
- **"Protecting"** is the Protect order (`W8_ACTION_PROTECT`).
- **On the move and in formation**: retail loses the 4 only when the target is
  in formation, as built.
- **A monster's spells, tiredness and waiting** are parts, counted once and
  capped out of formation. Retail counts them twice at a place; we keep one
  rule for every actor (rules/arms.md says so).
- **Places keep their `cover`**, though a member's is rewritten when gear
  changes: a place is what covers it.

### Still open

- **A monster striking a monster.** It lands by the target's own chances, as
  built; not looked up in the original.
- **What a monster's places are called** (by body type, six of them) for the
  hit messages. Add a name to `HitPlace` when the messages need it.
- **Which fatigue bands** the five penalties belong to, in our terms.

## Not designed yet

Everything but armour: attributes, skills, the maxima, resistances, attacks,
mending, damage reduction, missile shield; what a calling writes as sources;
drafts as entities; casting and combat without sheet fields on `Body` and
`Means`. Each is to go through the same steps: the rule in words from the
original, then components, then systems.
