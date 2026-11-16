# A member's arms and armour

What a member strikes with and how well they are guarded follow from what is
in their two hands and on their body. This page is the specification for the
last phase of slice 3 of [the plan](../plan.md); it replaces `Grip` and the
member half of `combat/attack.rs`. The formulas are the original's
(`CalcAttacks`, `CalcArmorClasses`) and are cited beside the code that holds
them; the shape below is not the original's.

## What a thing in a hand is

When an item is made, its definition says what it is to a hand, and that
becomes components. A rule never asks the definition again.

| Component | The fact it holds | On which items |
| --- | --- | --- |
| `Weapon` | The skill it is fought with; whether it strikes close or at range; its reach, dice, the ways it strikes, what its blows do besides damage, what it slays; what it adds to the rating to hit, to initiative and to the swings of an attack; how much strength helps it (fully, by half, not at all); whether an attack with it is ever more than one swing | what strikes |
| `Launcher` | It shoots what the other hand holds, and what it adds to the damage of that | bows, crossbows, slings, the omnigun |
| `Missile` | What it adds to a launcher's dice, effects, slaying and rating; whether it is used up when shot or thrown | arrows, bolts, stones; what is thrown |
| `Shield` | It guards rather than strikes | shields |
| `Protection` | Armour it adds where it is worn, resistance by realm, what it gives back of health, stamina and spell points | what protects |

Three of the original's lists become facts set once, when the item is made:
the weapons strength does not help (power glove, phaser, musket, omnigun,
rocket launcher), the one that only ever swings once (rocket launcher), and
the launchers.

## A hand

Each of the two hands of the weapon set in use is one of:

- **empty**: a fist, fought with Martial Arts. Its dice grow with the skill
  (two more in the main hand), it punches, and kicks past Martial Arts 4; a
  strong fist may knock out.
- **a weapon**: it strikes by its own components.
- **a launcher with its missile in the other hand**: it strikes with the
  missile's dice, effects and slaying added to its own. A launcher without
  its missile, or a gun out of shots, cannot strike.
- **a shield, a missile, or anything else**: it does not strike.

The off hand strikes only beside a main hand of the same kind (both close, or
both at range), never when the main hand holds an omnigun, and an empty off
hand does not strike beside an armed main hand.

Two hands that both hold weapons and both strike are **dual wielding**, which
costs each hand rating and swings by how far Dual Weapons falls short.

## An arm

Each hand that strikes is one arm on the member's `Attacks`. For each, the system
that writes `Attacks` works out, from the hand, the member's attributes and skills, and their load:

- **rating to hit** and the sheet's **to hit**;
- **attacks a round**, and **swings** an attack;
- **initiative**;
- **damage**: the dice, and the percent strength adds or takes;
- what its blows do besides damage, what it slays, the skill that may make a
  blow a kill, whether a throw always gets through armour;
- the skill that practising it trains.

Which arm opens an attack is the one with the most attacks, then the greater
initiative. A Berserk order needs the trait, an arm, and a main hand for close
quarters.

## Armour

From `CalcArmorClasses` (`GameplayCode.cpp`) and, in `Combat Attack.cpp`,
`GetTargetArmorClassModifier`, `GetTargetArmorClass`,
`TargetArmorClassAtLocation` and `BlockedForSpecialReason`. Higher is
better guarded; each point is five off a blow's aim and penetration.

### The parts

An actor's armour is a list of named parts that add up, as the character
screen lists them.

| Part | Who has it | What it is |
| --- | --- | --- |
| Race | all | 2 for a Faerie (the race table); a monster's armour from its record |
| Speed | members | -2 below 10, -1 below 20, +1 from 80, +2 from 90 |
| Stealth | members | a tenth of the skill |
| Reflextion | members | a twentieth of the skill plus 1, if the skill is usable; listed only when not zero |
| Shield | members | what the shields worn are worth; if above zero, Shield skill adds a 25th of itself, up to the shields' worth (a 15th, up to one and a half times, while defending) |
| Magic items | members | what the other guarding things worn are worth |
| Magic spells | all | what spells laid add against being struck |
| vs Penetration | all | what spells laid add against being pierced; not in the total |
| Conditions | all | what conditions take |
| Encumbrance | members | -1, -2 or -4 for a medium, heavy or extreme load |
| Fatigue | all | a tenth of what tiredness takes from an aim; lost by a member, gained by a monster |
| Defensive action | all | +2 for a member ordered to Defend, for a monster that waits (the original also gives it for Protect, which a member cannot order here) |

The guarding things worn are those in the two accessory places, as a cloak and
in the two hands in use; the stored weapon set counts for nothing.

- **Total**: every part but vs Penetration. One out of formation is guarded no
  better than -5.
- **Pierced**: vs Penetration, counted only where a blow lands.
- **Dead or worse than unconscious**: only Shield (without the skill) and
  Magic items count.
- **At a place**: the total, plus pierced, plus what covers the place.
- **Average** (the portrait): the places weighed by how often a blow lands
  there, rounded to the nearest.

### Places

- A member has five: head, torso, legs, hands, feet, hit 15, 40, 30, 10 and 5
  times in a hundred ([hit_places.ron](../../assets/tables/hit_places.ron)).
  What is worn on that part covers it. A monster striking a member lands by
  its own five chances instead (`attack_body_part_chances`).
- A monster has seven, each with a chance and a cover, decoded from its record
  as the species is read: the cover is that place's armour less its overall
  armour, so "total plus cover" is the one formula for both. A monster lands
  on a monster by the target's own chances.
- A blow the chances do not place lands on the trunk: a member's torso, a
  monster's fourth place.

### What a blow does with it

The blow's situation is what the target's armour comes to against this one
blow. It counts once against the aim and twice at a place (retail adds
`GetTargetArmorClassModifier` to the base and again at the location):

- caught unaware (or struck by a natural attack): a member loses its Speed and
  Shield parts, each only if positive; a monster vulnerable from behind loses 2;
- a member praying while the party is not running: 4 worse (there is no
  Pray order here yet);
- on the move (a member while the party runs, a monster advancing) and in
  formation: 4 worse, except against a throw or a shot.

Against a mode, a place is also less what the target has against that mode
(a monster's record; nothing for a member).

**Blocked**: a close blow that misses by no more than the target's block is
told and heard as blocked instead of missed; it does nothing either way. A
member's block is five times the Shield part, none when caught unaware; a
monster vulnerable from behind (the same record flag) blocks by 10, caught
unaware or not. Others never block. (In the original an unblocked miss may
also make a wounded member cry out and a facing monster dodge; neither is
here.)

### Where retail differs and we do not follow

Retail keeps a monster's spells, tiredness and waiting in the situational
modifier, so they count twice at a place, and outside the -5 out-of-formation
cap. Here they are parts like a member's: counted once, and capped. One rule
for every actor; the difference is a point or two at a place against a
monster that is tired, enchanted or waiting.

## When this is worked out

Only in the systems that write `Attacks` and `Armour`, which follow a change to what is worn or to a worn thing
(a launcher out of shots), and to the member's other sources ([actors.md](actors.md),
"Sources and figures"). Rules in a fight read `Attacks` and `Armour`.

## What goes

`Grip`, `Holds`, `is_launcher`, `NO_STRENGTH`, `armour()` taking a definition
lookup, `Character::resistances` taking one, and every `content.items[..]` in
`actors/stats.rs`, `combat/attack.rs`, `combat/swings.rs`,
`characters/resistance.rs`. What stays in `combat/attack.rs` is what a swing
rolls (aim, location, fumble, scatter), which is the round's and belongs to
slice 6.
