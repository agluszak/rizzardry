# Actors, from scratch

Written 2026-11-12, where the work stopped. Nothing here is built. It replaces
the approach of `docs/redesign.md`, which reshaped the existing code and was
rejected.

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

## Armour, as the original has it

From `CalcArmorClasses` (`GameplayCode.cpp`), `GetTargetArmorClass`,
`TargetArmorClassAtLocation` and the situational modifier in
`Combat Attack.cpp`, and the armour panel in `ReviewCharacterScreen.cpp`.

An actor's armour is a list of named parts that add up. The character screen
lists them, and blows read some by name.

| Part | What it is |
| --- | --- |
| Race | +2 for one with the Faerie armour trait |
| Speed | -2 below 10, -1 below 20, +1 from 80, +2 from 90 |
| Stealth | a tenth of the skill |
| Reflextion | a twentieth of the skill plus 1, if the skill is usable; listed only when not zero |
| Shield | what the shields worn are worth; if above zero, Shield skill adds a 25th of itself, up to the shields' worth |
| Magic items | what the other things worn are worth, apart from those at a body place |
| Magic spells | what spells laid add |
| Conditions | what conditions take |
| Defensive action | +2 while defending or protecting; Shield skill then adds a 15th, up to one and a half times the shields' worth |
| Encumbrance | -1, -2 or -4 for a medium, heavy or extreme load |
| Fatigue | 0, 0, -1, -2 or -4 by how tired |
| vs Penetration | a matchup bonus; not in the total, added at each place |

- Total: every part but vs Penetration. One out of formation is guarded no
  better than -5.
- Worse than unconscious: only the worn parts count.
- At a place: the total, plus vs Penetration, plus what covers that place.
- The things worn that count toward Shield and Magic items are those in the
  two accessory slots, the cloak and the two weapon slots in use. The stored
  weapon set counts for nothing.

What a blow does with it:

- One caught unaware loses the Speed part and the Shield part, each only if
  positive.
- A character praying is 4 worse.
- A blow that beats armour by no more than five times the Shield part is
  blocked, unless the target was caught unaware.

A monster has the same thing with fewer parts: its record's armour overall,
by place and by attack mode; Magic spells, Conditions and Fatigue as above;
+2 while it waits; -2 when caught unaware if it is vulnerable from behind; no
better than -5 out of formation.

Places:

- A character has five: head, torso, legs, hands, feet, hit 15, 40, 30, 10
  and 5 times in a hundred. A monster attacking a character uses its own five
  chances instead (`attack_body_part_chances`).
- A monster has seven hit locations of its own, each with a chance and an
  armour value from its record. What a location is called depends on the
  monster's body type (six of them). A location with a chance of 100 is not
  named in the message.

## Components

Not compiled.

```rust
/// A part of an actor's armour, as the character screen lists them.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmourPart {
    Race,
    Speed,
    Stealth,
    Shield,
    MagicItems,
    MagicSpells,
    VsPenetration,
    Encumbrance,
    Conditions,
    Fatigue,
    DefensiveAction,
    Reflextion,
}

/// The parts of an actor's armour. Each is written by what it comes from.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ArmourParts(pub EnumMap<ArmourPart, i32>);

/// The places of a body where a blow can land.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Places(pub Vec<Place>);

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// What the place is called when a blow lands there, if it is named.
    pub name: Option<PlaceName>,
    /// How often a blow lands here, in a hundred.
    pub chance: u32,
    /// What covers it, beyond the parts.
    pub cover: i32,
}

/// What it has against each way of attacking.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Against(pub EnumMap<AttackMode, i32>);

/// How well it is guarded: the parts together, and at each of its places.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Armour {
    pub total: i32,
    /// One for each of its `Places`, in their order.
    pub at: Vec<i32>,
    pub average: i32,
}

/// One caught unaware is 2 worse guarded.
#[derive(Component)]
pub struct VulnerableFromBehind;
```

## Who writes what

| Data | Written by | When |
| --- | --- | --- |
| `Race` part | where the actor is made: a species' armour, 2 for a Faerie | once |
| `Speed` part | a system on `Attributes` | Speed changes |
| `Stealth`, `Reflextion` parts | a system on `Skills` | skills change |
| `Shield`, `MagicItems` parts, a character's `Place::cover` | a system on what is worn (Shield also reads the skill and whether it defends) | gear changes |
| `MagicSpells`, `VsPenetration` parts | a system on spells laid | a spell comes or goes |
| `Conditions` part | a system on `Conditions` | a condition comes or goes |
| `Encumbrance` part | a system on `Burden` | load changes |
| `Fatigue` part | a system on stamina | tiredness changes band |
| `DefensiveAction` part | a system on the action taken in a fight | defending, protecting or waiting begins or ends |
| `Places`, `Against` | where the actor is made: a table for a character, its species for a monster | once |
| `Armour` | one system: the parts but `VsPenetration` summed, no better than -5 out of formation; each place the total plus `VsPenetration` plus its cover | any of the above changes |

A monster's species gives each place's cover as the difference between that
place and its overall armour, so "total plus cover" is the one formula.

The screens list `ArmourParts` and show `Armour`. The creator's draft is an
actor like any other, so it has both.

## Open

- **A monster attacking a monster.** A monster's own five chances only fit a
  five-place body. What the original does when its target is a monster has
  not been looked up. Do this before designing the blow.
- **`Armour::at` parallel to `Places`.** Two lists that must agree in length.
  Keeping the result on each `Place` avoids that but mixes a source with a
  figure.
- **Worse than unconscious.** Either the `Armour` system leaves the unworn
  parts out of the sum (the parts stay true, the screen shows parts that do
  not count), or each owner writes zero.
- **The modifier counted twice.** At a place the original adds the
  situational modifier (unaware, praying, a monster's spells and tiredness)
  twice. It belongs to the blow; decide there.
- **Race armour.** The original gives 2 by the Faerie trait. Our race table
  has an armour value per race. Not checked which is right.
- **"Protecting".** Assumed to be the Protect order (`W8_ACTION_PROTECT`).
- **Which fatigue bands** the five penalties belong to, in our terms.

## Not designed yet

Everything but armour: attributes, skills, the maxima, resistances, attacks,
mending, damage reduction, missile shield; what a calling writes as sources;
drafts as entities; casting and combat without sheet fields on `Body` and
`Means`. Each is to go through the same steps: the rule in words from the
original, then components, then systems.
