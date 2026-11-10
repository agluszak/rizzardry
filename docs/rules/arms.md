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

Each hand that strikes is one arm on the member's `Attacks`. For each, refit
works out, from the hand, the member's attributes and skills, and their load:

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

A member's armour is, everywhere: what Speed gives or costs, two for a Faerie,
less what their load costs; what is worn in the two miscellaneous places, as a
cloak and in the hands adds; a shield's worth and the skill to use it. Each
place of the body adds what is worn there. Bracing (defending) adds two
everywhere and lets Shield skill count for more. A blow from behind gets past
what Speed and the shield add.

Resistance and regeneration sum what is worn in use.

## When this is worked out

Only in refit, and refit is fired by the `Worn` relationship changing, by a
missile stack running out, and by the events that already fire it (a level, a
skill point, a spell, a load). Rules in a fight read `Attacks` and `Armour`.

## What goes

`Grip`, `Holds`, `is_launcher`, `NO_STRENGTH`, `armour()` taking a definition
lookup, `Character::resistances` taking one, and every `content.items[..]` in
`actors/stats.rs`, `combat/attack.rs`, `combat/swings.rs`,
`characters/resistance.rs`. What stays in `combat/attack.rs` is what a swing
rolls (aim, location, fumble, scatter), which is the round's and belongs to
slice 6.
