# Actors

An actor is anyone who can be hurt and can act: a party member or a monster.
Rules read an actor's numbers and never ask which of the two it is.

## What an actor has

| Component | What it is |
| --- | --- |
| `Attributes` | Strength, Intelligence, Piety, Vitality, Dexterity, Speed, Senses, each 1–125, as they stand now with spells counted. |
| `Level` | The level saves, resistance and special attacks weigh. |
| `Vitals` | Health and stamina, and for now what ails it and the spells on it (slice 2 moves those out). |
| `Resistances` | Percent resistance in each realm, the conditions it cannot suffer, whether it can be slain outright, its creature kind if it has one, and how strongly it casts from each book. |
| `Armour` | Armour against being struck at all and at each place a blow can land, what a surprise strips from it, what bracing adds, the percent cut off damage, the chance a missile is turned aside, and where blows land on it. |
| `Attacks` | Each thing it can strike with (reach, modes, dice, rating, effects, attacks and swings a round), when it acts in a round, how hard it presses past a guard, and how it fares blind or from hiding. |
| `Side` | Friendly, neutral or hostile to the party. |

An actor is made with all of them. A rule that needs a number an actor lacks is
a bug in how that actor was made, not a case for the rule to handle.

## Where the numbers come from

They are written when the actor is made and again whenever it is **refitted**.
Nothing else writes them, and nothing recomputes them between refits.

A **monster** is made from its species record:

- attributes from the record (it has no Piety or Vitality; those read as 1);
- level, kind, immunities by kind, resistances, hit locations, armour by place
  and by attack mode, damage reduction and initiative straight from the record;
- armour is 10 less the record's value at each place, 10 less its evasion
  overall, so that lower is always better guarded;
- each of the record's attacks is one thing it strikes with, at the record's
  attacks and swings a round.

A **member** is made from their sheet and what they wear:

- attributes from the sheet;
- level is the sum of their profession levels;
- resistances from race, realm skill, Iron Will, high Intelligence (Mental) and
  Piety (Divine), and what is worn;
- immunities from traits (an Android's, the Fearless, a Psionic's mind);
- armour from Speed, race, load, a shield and the skill to use it, and what is
  worn at each place;
- one thing to strike with per weapon slot in play, rated by weapon skill,
  attributes, load and the weapon.

Formulas for each are beside the code that applies them, with the routine of
the original they were read from. Where the original computes the same number
differently for a member and a monster (initiative, armour, to-hit), both
formulas are kept; the difference is which one fills the component.

**Spells laid on an actor**, and on the party for its members, are counted at
refit: Armorplate, Bless, Body of Stone and Armormelt on armour; Bless and
Enchanted Blade on rating; Magic Screen, Element Shield and Soul Shield on
resistance; Missile Shield on missiles; Superman and Haste on attributes.

## What refits an actor

An actor is refitted when one of its sources changes, and only then:

- it is made, or joins or leaves the party;
- a member puts on, takes off, uses up or loses something worn;
- a member's sheet changes: a level, an attribute, a skill point;
- a member's load band changes;
- a spell that counts above is laid on it or lifted from it, or on or from the
  party for a member.

Whatever changes a source says so by firing `Refit` at the actor. Being struck,
healed, tired, or afflicted with a condition does not refit anyone.

## What is read as it stands

Some things move too often to be written down and are read by the rule that
needs them:

- **fatigue**, from stamina spent: it takes from aim, from pressing past a
  guard, and adds to a monster's armour;
- **bracing**, from defending this round;
- **being on the move**, from walking or the party running this round;
- **where it stands**.

## Not yet in this model

`Side` and where an actor stands are still re-derived each step; conditions and
enchantments are still fields of `Vitals`. The plan's later slices change those.
