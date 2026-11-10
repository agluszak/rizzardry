# Casting a spell

This page says what happens when an actor casts a spell, or uses an item that
casts one, in the game's terms. It is the raw material for the redesign of the
casting code: complete and exact about what happens, silent about how the code
is organised. Numbers and formulas are the original's and are cited in
parentheses beside them. Where the original was looked up and the current code
does something else, the text says what the code does and the *Open questions*
section says what the original does; the code is not fixed here.

What each spell does to a victim is a table (`assets/tables/spell_effects.ron`,
see [tables](tables.md)); this page does not restate it. It says how a row of
that table is applied. What an actor is, what hurts it and what ails it is on
[actors](actors.md); spells laid on a bearer are there too. When an actor's
moment comes is on [round](round.md). Items are on [items](items.md). Weapon
attacks, which share some words with this page (reach, press, provocation), are
on [attack](attack.md).

Throughout, "roll d100" is a whole number from 1 to 100, "below(n)" is a whole
number from 0 to n-1, "dice" are `count` dice of `sides` sides plus `base`, and
every division is whole-number division unless said otherwise. A spell's own
figures (realm, level, point cost, books, dice, rounds, radius, reach, whom it
is aimed at, when it may be cast, whether a monster may cast it, whether it
flies) are its record in the catalogue, from the original's spell database.
The *reaches* are touch 2 m, short 4 m, long 25 m, extreme 50 m, measured
between the edges of bodies.

## 1. What a cast is

A cast is **one spell, cast by one source, at one aim, at one power**, once.
The sources are:

- a member's **learned spell**, paid for in spell points;
- a member's **item**, which carries a spell at a power of its own and is paid
  for in uses or charges;
- a monster's **own spell**, which costs it no points (it is powered by its
  budget, section 3);
- no one: a sprung trap, a cloud's pulse, a burst, a spell turned back.
  These land as casts with no caster (sections 6 and 8).

In a fight a member's cast is their order for the next round, accepted as the
round begins, and it is made at their moment ([round](round.md)). A monster
decides at its moment ([monsters](monsters.md)). Outside a fight a member's
cast happens at once. A cast made in a fight counts as an attack for deciding
whether the fight has gone passive. A cast takes the actor's moment and holds
the fight for the pause of one second after it.

A cast goes through these steps, in this order:

1. It is **refused** (section 2) and nothing is spent; or it goes on.
2. It is **aimed** (section 5): whom it reaches, and where it lands.
3. Its **power** is settled (section 3).
4. A member's cast against enemies **provokes** them (section 7).
5. It is **rolled** (section 4): it comes off, fizzles or backfires.
6. It is **paid for**, whatever the roll (section 7): points, a use, stamina.
7. A cast that came off is **released** at its victims: at once, or, for a
   spell that flies, as each missile arrives.
8. On landing, each victim takes each effect of the spell in turn (section 6);
   then what follows the whole cast is settled: reflection, bursts, drain-back.
9. The caster **practises** (section 7), and the player is told.

A member whose cast was refused defends instead for that round
(retail `ExecuteCharacterAction`: an action that fails falls back to Defend).

## 2. Who may cast what, when and where

A cast is refused, and nothing is spent, for the first of these that holds:

1. **Cannot act.** The caster must be able to take a combat action: alive and
   with nothing as bad as Asleep ([actors](actors.md) *Conditions*). Only a
   party member may cast, except for a monster's own spell and a cast with no
   caster.
2. **Silenced.** A cast that is voiced is refused while the caster is Silenced:
   a learned spell, or an instrument. Any other item is not voiced and is not
   stopped. A silenced member can still cast an Alchemy spell, one in the
   Alchemy book, if their Alchemy skill is above 0
   (`IsSpellBlockedForCharacter`); an instrument is stopped whatever it plays.
   A monster is stopped by Silence unless the spell is an Alchemy spell and the
   monster is of one of the kinds that keep their Alchemy (Ranger, Alchemist,
   Ninja; `IsSpellBlockedForMonster`).
3. **Not learned.** A learned spell must be one the member has learned.
4. **Power 0.** A learned spell chosen at power 0 is refused.
5. **No spell points.** A learned spell needs points for at least power 1 in
   its realm (section 3).
6. **An item.** The item must still be there, carried by the caster or, outside
   a fight, in the party's pool (in a fight the pool is out of reach,
   `CanPartySlotUseRecordedItem`). It must carry a spell, of a power above 0 (an instrument or
   gadget has none of its own, section 3). The user must be one who may
   activate it: not an item with no use, nor a spellbook; worn in a slot in use
   if it must be worn (weapons, armour, misc worn things, and anything marked);
   usable by their profession, race and sex, except that **potions and food are
   usable by anyone** who holds them. A charged item with no charges left is
   refused as having no charges.
7. **Not resolved.** The game must resolve the spell: it has a row in the
   table, or it is one of the utility spells of section 8, or it is Divine Trap
   or Knock Knock.
8. **Wrong time.** The moment must suit the spell, by where the party is:

   | The party is | A spell may be cast if its record says it is usable |
   | --- | --- |
   | in a fight | at any time, or in combat |
   | exploring | at any time, or out of combat |
   | camped | at any time, out of combat, or while camped |
   | talking with someone | only Charm and Mindread |
   | picking a lock | only Knock Knock |
   | disarming a trap | only Divine Trap and Knock Knock |

   A spell usable only on a lock or trap is never cast otherwise. The "repeat
   until whole" choice of power (section 3) is refused in a fight. While a lock
   or trap attempt is under way, no other spell is cast. Summon Elemental is
   refused unless there is a fight; and refused while the elemental the caster
   summoned still stands.
9. **No target** (section 5): nothing can be aimed at.

The original also admits in a camp only the spells usable while camped, with
Identify Item and Remove Curse always allowed there; and refuses Set Portal
where the ground is not fit. See *Open questions*.

What is **not** asked: a member need not hold a weapon or a free hand; a
spell may be cast with stamina at 0 (it then tires the caster out, section 7).
Reach to the aimed body is asked when aiming, not here.

## 3. Power: what it may be, what it costs, a monster's budget

**Power** is a whole number from 1 to 7.

### A member's learned spell

The member chooses a power, or asks for the **auto** power (retail "8", "as
affordable"). The power cast is the choice cut down to what the realm's spell
points pay for: `power = min(choice, points left in the spell's realm / cost)`.
A spell that costs `cost` points per power, cast at `power`, costs
`cost * power` points. If the points pay for none, the cast is refused. A
spell whose record fixes its power is cast at 1 whatever is asked (retail
`CastSpellFromSource`, `Magic.cpp:2527-2563`).

*Spell points* are kept per realm (Fire, Water, Air, Earth, Mental, Divine). A
member has points only in a realm in which they have learned a spell.
Maximum of a realm: with the four book skills sorted high to low,
`weight = min(125, best + second/2 + third/4 + fourth/8)`; then
`(weight + 3 * realm skill + Piety) * 2 / 100 * (spells learned in the realm +
level + 1) + 0.5`, cut down; never below the cost of the dearest learned spell
in the realm. Spent points come back evenly as time passes, at
`(0.65 * maximum + 20) / 240` points an aging tick (half that rate at least),
the realm that is most short first (retail `RecalculateRealmSpellPoints`,
`RestoreCharacterSpellPointsEvenly`, aging). A realm's current points are its
maximum less what has been spent, so a drop in the maximum leaves a deficit
that a later rise does not refill.

**Auto power** is chosen for the aimed victim, never above what is affordable
(retail `ChooseSpellPowerLevelForTarget`):

- *Heal Wounds, Restoration, Stamina* on a member: if nothing is missing, 1.
  Otherwise the lowest power whose average roll exceeds what is missing, but
  stopping one power short as soon as the failure chance at a power goes over
  10 percent. The average of the spell's dice at power `p` is
  `(base + count*p*sides + base + count*p) / 2`, cut down. What is missing is
  health for Heal Wounds; stamina for Stamina; health, or stamina if health is
  full, for Restoration. (`ChoosePowerLevelToRestore`)
- *The cures* (Cure Lesser Conditions, Cure Paralysis, Cure Poison, Cure
  Disease, Sane Mind, Remove Curse): the power that cures the longest of the
  conditions the spell cures most cheaply. For each power `p` from 1 up to what
  is affordable, `per = p*p - failure(p) * p*p / 100`; `casts = ceil(turns /
  per)`; the price is `cost * casts * p` points; the cheapest wins, the lower
  power on a tie. A condition without end is answered with power 1. Remove
  Curse also covers the binding of the victim's worn items, if that asks for
  more. (`ChoosePowerLevelForDuration`)
- *Identify Item*: the least power that reveals the item (section 8).
- Everything else, including Heal All and Rest All: 1.

Outside a fight the "repeat" spells (those whose record offers it) may be cast
at auto power as an **auto recast**: after it lands, the caster casts it again
while any victim is short in what it restores and the caster has stamina above
the spell's level and points for power 1. A cast that fizzles is also cast
again at once, without provoking anyone anew (retail `ExecuteCharacterAction`:
the loop on result 2 while power is 8 and the fatigue and points hold).

### A member's item

The item's spell is cast at the power in the item's own record. An item
without a record power is refused, except an **instrument** or **gadget**,
which is *played*: the player tries powers from 7 down, each time asking the
item's failure chance at that power (section 4) and stepping down one power
while it is above 15 percent; at the first power at or under 15 percent,
starting from `chance = 7 * power`, steps further down while `chance >
below(100)`, taking 7 off the chance each step. If the power reaches 0 the
instrument is **played badly**: the attempt tires the player, trains the skill
once, costs nothing else, and is refused with that word (retail `UseItem`).

### A monster's budget

A monster pays no points. In place of a caster's skill it has a **budget**:
its species' budget plus whatever it has gained or lost since (Might to Magic
gives it half of what it drained; Draining Cloud and drains take it away; never
below 0). Its power is chosen as follows (retail `ChooseMonsterSpellPowerLevel`):

1. Start at power 1. While the failure chance of the spell at that power with
   the budget as skill (section 4) is under 9 percent, and the power is under 8,
   raise it by 1.
2. If it is above 1, take 1 off.
3. Roll below(4): on 0, take 1 off if it is above 2; on 1, add 1 if it is
   below 7; otherwise no change.

### The strength a cast goes off at

What a victim's save weighs against is the cast's **strength**, worked out from
the figure of whoever the cast comes from (retail `GetSpellDifficulty`):

```
against = figure / 2 + power + (cost / 2 + level) / 2
against = against - 3        if the spell is aimed at all enemies
against = against - floor    Instant Death 2, Quicksand 4, Asphyxiation,
                             Death Cloud, Death Wish 6
against = max(against, 0), then  + against * (boost / 2) / 100
```

The **figure** is: for a learned spell, the member's total caster level (the
current profession's caster level plus that of every other profession they
have held that shares a spell book with the spell; section 4); for an item,
the user's level, or their Bard levels for an instrument, their Gadgeteer
levels for a gadget; for a monster, its level; for a cast with no caster, 0.
The **boost** is the member's Power Cast bonus, `skill / 4 + 1` while the skill
is available to them, otherwise 0; it applies to learned spells only. The boost
also raises every dice and duration roll of the cast by that percent
(section 6). An item's cast has no boost.

A spell coming from someone else than its caster (a backfire, a reflection, a
burst) is as strong as *they* are: the figure is the owner's highest caster
level among the spell's books (a monster's level; a member's total caster
level), 0 if no owner is left; the power and the boost are those of the cast.

## 4. The roll to cast

### The failure chance of a learned spell

```
skill  = (book skill + 4 * realm skill) / 5
need   = threshold[band] * power / 7        band = min(cost/2 + level, 16)
chance = (need - skill) * 70 / need         if skill < need, else 0
short  = least caster level for the spell's level - total caster level - 1 + power
chance = chance + level * short             if short > 0
chance = chance + 50                        Sane Mind cast by a member on themselves
```

with the thresholds for bands 0 to 16: 30, 40, 50, 58, 64, 70, 76, 81, 86, 90,
94, 97, 100, 102, 105, 107, 110 (retail `GetSpellFailureChance`,
`ExecuteCharacterSpellCast`). The least caster level for spell level 1 to 7 is
1, 3, 5, 8, 11, 14, 18. The **book skill** is the skill in the spell book the
member has most points in among the spell's books; the **realm skill** is the
skill in the spell's realm (Fire Magic and so on).

A profession's **caster level** is its level plus an offset: 0 for Priest,
Alchemist, Bishop, Psionic, Mage; -4 for Lord, Valkyrie, Ranger, Samurai, Ninja,
Monk; none (it cannot cast) for Fighter, Rogue, Gadgeteer, Bard. The books a
profession casts from: Lord, Valkyrie, Priest Divinity; Ranger, Ninja,
Alchemist Alchemy; Samurai, Mage Wizardry; Monk, Psionic Psionics; Bishop all
four (retail `g_profession_spellbooks`, `GetProfessionCasterLevel`,
`GetTotalCasterLevel`).

In a fight the chance is also scaled by how late the caster's moment is
(retail `ScaleByCombatPace`). With `pace` = 80 on Novice, 60 on Normal, 40 on
Expert, and `moment` the caster's moment in the round (1 to 100; 0 outside a
fight): if `moment >= pace`, `chance = (50 - pace + moment) * chance * 2 / 100`;
otherwise unchanged. Nothing caps the chance, before this or after: a chance
past 100 counts for a third of itself in the chance to backfire (below). Sane
Mind cast by a member on themselves adds its 50 after the pace.

### The failure chance of an item

The skill is the user's skill that presents the item's spell: Artifacts for
charged items and scrolls, Throwing and Sling for thrown items, Music for
instruments, Engineering for gadgets, Modern Weapon for flamethrowers. Items
with no such skill never fail. Else, with `level` the skill's level:

```
chance = spell failure chance at skill = level, as above (no caster-level term)
thrown:               chance = chance * 8 / 10
charged, scroll:      chance = chance * 9 / 10
instrument, gadget:   chance = chance + (15*spell level - level) / 3   if level < 15*spell level
                      chance = chance + spell level * adjust
                      adjust = least caster level - Bard (or Gadgeteer) levels - 1 + power, if > 0
chance = scaled by pace (as above)
chance = chance + 30                          if the item is not identified
thrown: chance = chance / 2 (a "quiet" use)
```

(retail `GetItemUseDifficulty`, `CastItemSpell`). It is not held at 100. How well
an instrument or gadget is played (section 3) asks the item's chance without
the pace.

### A monster's failure chance

`chance = spell failure chance with the budget as the skill`, with no
caster-level term and no pace (retail `MonsterCastsSpell`).

### The roll

If the chance is 0 the cast comes off. Otherwise roll `r = below(100)`; the cast
**fails** if `r < chance`. Two things first change the chance: a Fire spell
cast under water has chance 100 (retail `g_camera_sway_active`: the party is
under water); a quiet use (thrown) halves it, after that.

A failure is a **backfire** if the spell *can* backfire and `r < backfire`,
and a **fizzle** otherwise:

- `backfire = chance / 3`, and 0 if the chance is under 6;
- for a quiet use, `backfire = chance`;
- under water, a Fire spell never backfires.

A spell **can backfire** (retail `CanSpellBackfire`) if it is aimed at enemies
(an enemy, a group, a cone, a radius, all enemies) other than Banish, Boiling
Blood Explosion and Rocket Blast; or it is one of Stamina, Cure Lesser
Conditions, Cure Paralysis, Rest All, Haste, Remove Curse, Sane Mind, Charm; or
it is Knock Knock. No other spell backfires.

A **fizzle** does nothing at all. Both fizzle and backfire are paid for.

### What a backfire does

The cast is turned on its caster (retail `PrepareSpellTarget`,
`RedirectBackfiredSpellTarget`). The aimed body becomes the caster of it and
the caster the victim; its strength is the aimed body's:

- aimed at one body (an enemy, a friend): the aimed body casts it back on the
  caster; except Charm and Mindread, whose backfire is told in section 8;
- aimed at a group: the group's leader casts it, and it falls on the caster's
  side: the whole party if a member cast it, the caster's own group if a
  monster did, however far or out of sight;
- a cone or a blast: it falls on the caster's side where the spell was to go
  off (a blast centred on the caster, a cone toward the aimed spot);
- aimed at all enemies, the party, or an item: it is cast as it was, but
  falls on the caster's own side.

What the backfired spell does to those it reaches is its **backfire effect**
(retail `ProcessSpellEffectTargets` with the backfire flag):

| Spell | Its backfire does |
| --- | --- |
| a spell aimed at enemies | what it always does, to the other side |
| Stamina, Rest All | tires, by the spell's dice, instead of restoring |
| Cure Lesser Conditions | one of Irritated, Nauseated, Afraid, Asleep, Blind, by below(5) |
| Cure Paralysis | paralyses (with the spell's rounds and save) |
| Sane Mind | makes insane |
| Remove Curse | hexes |
| Haste | slows |
| Charm, Mindread | section 8 |
| Knock Knock | section 8 |

A backfire never flies as a missile. A backfired cast told to the player as
such, once; a quiet use is not told twice.

### The aim of a blind caster

A blind caster's aim goes astray (retail `CheckSpellBackfire`): unless the
spell is aimed at themselves, is of the Mental realm, is Rocket Blast, is a
reflection or an auto recast, a Blind caster who does not have the Effective
While Blind trait (a monster: is not a Monk) has a 20 percent chance that the
aim is changed. A member with the trait escapes it with chance 50 scaled by
profession level, the rest of the time it is the same 20 percent. If it goes
astray, a spell aimed at one body, a friend or a group takes a random other
body in the fight (members and monsters alike, not the caster; for a group
spell, that body's whole group or the party); a cone, a blast or a point
spell goes to a random spot within the spell's reach of the caster. The
player is told. The Rust code has no such rule (see *Open questions*).

## 5. Whom it reaches

A spell is aimed by its **kind of target**. A body is **reached** when it is
within the spell's reach of the caster, edge to edge, and there is a clear
line between the caster's eye and its eye (retail `CharacterActionReachesTarget`,
`SourceActionReachesTarget`). *Friends* are those the caster stands friendly
toward; *foes* those it stands hostile toward ([attack](attack.md) section 2).
The victims are taken in the order of the fight: the party's members by seat,
then the monsters as they joined it.

| Targeting | The victims |
| --- | --- |
| the caster | the caster |
| a friend | the body aimed at, if it is the caster or a friend, and reached |
| the party | every member of the party (the dead too: the rows decide) |
| an enemy | the foe aimed at; or, aimed "ahead", the foe reached whose direction from the caster's eye is nearest to the way the caster faces, the nearer of two equal; a foe must be reached |
| an enemy group | the foe aimed at, then every other of its group, reached or not (`PopulateSpellTargetMarkers`) |
| a cone | every living body but the caster, reached, inside the cone toward the aimed foe |
| a radius | every foe whose body is within the spell's radius of the spot (see below) and that has a clear line to the spot |
| all enemies | every foe reached |
| a point | the spot aimed at; nothing is a victim |
| an item | the item aimed at |
| a lock or trap | the attempt under way |

- A **cone** is the apex at the caster's eye. A body is inside it when the
  heading from the apex to it and its elevation each differ from those toward
  the aimed spot by less than 45 degrees (retail `TargetInRangeAndArcs`). Range
  is the spell's reach. The cone's victims in the Rust code include friends and
  foes alike (see *Open questions* 34).
- A **radius** is a blast at the aimed foe's feet. Its radius is the spell's
  radius plus its radius per power times the power, in metres. A body is within
  it when the distance from the spot to the body's centre, less the body's
  radius, is at most that. It reaches the foes of the caster only. A blast
  needs a power before its victims are known.
- An enemy aimed at but not in the list of foes reached is **no target**: the
  cast is refused. All-enemies and group spells with no foe reached are
  likewise refused. A spell aimed at all enemies is also refused if no foe is
  there; but see Sonic Boom (section 8).
- Outside a fight, **no monster is a foe**: the hostile spells are combat spells
  (but section 8, Lure).

### Missiles

A spell with a missile record **flies**. Its caster's report is told as it
leaves. Each victim has its own flight: from the caster's eye to the victim's
feet (to the spot for a blast), at 15 m/s (the original's missile speed of 30
per 2 ms step, retail `Missile.cpp`). The spell lands on each victim when its
missile arrives. The fight waits for what is in the air ([round](round.md)).
Once released, a flight lands even if the caster dies meanwhile or the item it
came from was used up. A party-wide effect (section 6) lands once, with the
first member a missile reaches. A backfire does not fly. A missile spell that
reaches nobody does not fly, and the caster practises as for a cast that came
off.

## 6. What lands

A spell lands on each victim as the **deed** it is: its effect rows, in the
table's order, one after another, each seeing what the one before it did. A
victim may take rows from a spell it already bears (they replace as
[actors](actors.md) says). A row has these parts: what it **does** (its kind),
whom it is **for** (everyone, members only, monsters only, the party as a
whole), how its **magnitude** is rolled, a **chance**, a **save**, a **realm**
it is resisted in, whether it is **half** strength, whether it waits on the
row before (**landed**, **resisted**), and which **kinds** of monster it is
for.

For one victim and one row, in order:

1. **Whom it is for.** A row for members is skipped on a monster, a row for
   monsters on a member; a row for given kinds on a body not of the kind. A row
   for the party is not for a victim at all (below). A row that waits on the
   one before it, which did not come out so, is skipped and counts as missed.
2. **The pick.** Prismic Ray and Prismic Chaos choose here: below(100); at 50
   or more it is damage by the spell's dice; otherwise it is a condition for the
   spell's rounds, chosen by weight from Nauseated 10, Afraid 10, Hexed 5,
   Insane 5, Blind 5, Turncoat 1, Asleep 10, Paralyzed 3, and death 1, with the
   save of that condition (death has the death save).
3. **The roll.** The magnitude is rolled now (below). Rolls are made before the
   save, as in the original.
4. **The dead.** A row that needs a living body (harm, drain, afflict, slay,
   tire, heal, refresh, replenish) does nothing to a dead one: missed.
5. **Chance.** A row with a chance happens with it, per victim: `below(100) <
   chance`; else missed.
6. **Immunity.** A condition the victim's kind cannot suffer is resisted:
   undead: Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep,
   Nauseated; android: Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep;
   plant: Insane, Afraid, Nauseated. A death that the victim cannot suffer is
   resisted likewise. (retail `g_condition_immunities`)
7. **The save.** A row with a save and a realm lets the victim save:

   ```
   chance = (victim's level - against) * 3 + save base + victim's resistance in the realm
   ```

   scaled for difficulty by the victim's side ([attack](attack.md) *Difficulty*),
   held within 5 and 95, and the victim saves if `below(100) < chance`. With
   *half*, `against` is halved first. `against` is the cast's strength
   (section 3). A member who saves practises Iron Will twice. (retail
   `TargetResistsCondition`) The save base of each condition: Drained 1,
   Diseased 2, Irritated 3, Nauseated 4, Slowed 4, Afraid 5, Poisoned 6,
   Silenced 7, Hexed 8, Infatuated 9, Insane 10, Blind 12, Turncoat 14,
   Webbed 16, Asleep 20, Paralyzed 18, Unconscious 22, Dead 24 (retail
   `g_condition_resist_base`).
8. **Resistance.** A row with a roll and a realm that was not resisted has its
   magnitude cut: `c = resistance + (victim's level - against) * 3`. If `c` is
   above 0: `c = c + 1`, `half = c / 2`, `share = min(100, below(c - half) +
   half + below(half))`, and `magnitude = magnitude - magnitude * share / 100`.
   A magnitude cut to 0 means the row is resisted. A permanent magnitude (9999)
   is not cut. (retail `ReduceMagnitudeByResistance`)
9. **Wards.** A row that allowed a save and afflicts a condition the victim is
   warded against (a member's traits) is resisted.
10. A resisted row is told as resisted and the next row sees it so. Otherwise the
    row is **applied**, and the next row sees it landed.

What "half" and "if resisted" mean, then: *half* halves the strength the save
and the resistance weigh against, and the rounds rolled (`InflictConditionAttack`); a row marked *landed*
needs the row before it to have landed on this victim (Terror's Unconscious
for members, Terror's and Sonic Boom's second Afraid for monsters); a row marked *resisted*
needs the row before it to have been resisted (Pandemonium's Afraid comes only
if the Insane was resisted).

The chosen magnitude is applied as the row's kind says. A victim killed by a
row (a Valkyrie may cheat death, [actors](actors.md)) is settled before the
next row.

### Rolling a magnitude

- **The spell's dice** (damage, healing): `count * power` dice of `sides` plus
  `base`, rolled and summed; then raised by `boost` percent. A row may take a
  multiple of the spell's dice, or the dice unboosted, or divided.
- **Rounds** of a condition or enchantment: `n = per power * power + base`
  from the spell's record. If `n` is 9999 it is permanent. Else `rounds = n + 1`
  (not below 0); then roll below(4): on 0 add one if `rounds > 1`; on 1 add one
  if `rounds <= 2`; then raise by `boost` percent. (retail `RollEffectDuration`)
- **Purify**: `base = power + 1`; below(4): on 0 add one if `base > 1`; on 1 add
  one if `base < 3`; raised by `boost` percent.
- **Gold share** (Philosopher's Bane): the spell's dice at power; if under 100
  rolled once more, taking the second, held at 100; else 100.
- **Fifty** (Anti-Magic): `50 * power`, raised by `boost` percent. **Per power**
  (Heatwave): `10 * power`.
- A fixed number.

### Applying each kind

- **Harm**: damage as [actors](actors.md) *Being hurt* says (Guardian Angel,
  fatigue, health). If the victim was asleep and survived, it wakes with chance
  `Senses / 2 + 50` percent. Spells have no damage reduction and no armour.
- **Sap** (members only): the victim's maximum health is drained until Drained
  is lifted.
- **Slay**: the victim dies outright (health to 0, no angel, no fatigue).
- **Afflict**: the condition for the magnitude in rounds; a magnitude of 1
  becomes 2; permanent if the row says so. A longer duration replaces a shorter.
  Turncoat on a neutral monster does nothing. Poison also takes the strength
  the row names. Slowed first spends the rounds against the victim's Haste (and
  Haste against Slowed) and what is left of the slowing is laid; if none is
  left, the victim is still told as slowed.
- **Tire**: stamina lost, by the magnitude.
- **Drain spell points**: a member loses the magnitude in points: from a
  random realm each draw, up to fifty draws, taking what each realm has; or from
  one realm named, or from every realm named in turn. A monster loses the
  magnitude from its budget.
- **Heal**, **Refresh**: health or stamina given, up to the maximum. The dead
  are not healed. **Replenish**: spell points given back evenly (the realm most
  short first).
- **Cure**: for each condition the spell names, if the victim bears it. A timed
  condition loses `power * power` rounds (raised by the boost); it is lifted if
  that is all it had left. A poison also weakens by the same share. A permanent
  condition, or any condition when the cure is *outright*, is lifted with
  chance `power * 15` percent (100 for outright). (retail
  `TryCureConditionOnTargets`)
- **Renew**: Drained is lifted from a member, giving back what draining took.
- **Raise**: a dead member lives again with `(Piety + Vitality) / 4` percent of
  their maximum health and stamina, not under 1 each. Only members are raised.
- **Purify**: the clouds laid on the victim lose `magnitude + purifier's level
  - cloud's level` turns.
- **Enchant**: the spell is laid on the victim for the magnitude in rounds
  ([actors](actors.md) *Spells laid*); Guardian Angel's absorption is rolled as
  it is laid, `dice * power`, raised by the boost. Haste first spends itself
  against a slowing. A magnitude of 0 lays nothing.
- **Burn**: each potion, scroll and spellbook the victim carries (each of a
  stack on its own) is lost with the magnitude in percent.
- **Lose gold**, **Drain spell points of the party**, **Enchant the party**,
  **Purify the party**, **Dispel**: see below.

### The party as a victim

A row for the party lands **once**, with the first member the cast reaches,
after that member's own rows. It lands on the party's shared things: its
purse (Lose gold: that percent of the gold, at most 100), the spells laid on
the party, and every living member's spell points (Drain spell points: each
member drains as above). **Dispel** ends the spells on the party that are not
lasting for the whole fight (the fight-only ones: Bless, Armormelt, the shields,
the clouds, Ring of Fire). A row for the party also waits on the row before as
any other.

### Reflection

After a spell aimed at enemies (not a backfire, not already reflected) has
landed on all its victims, the strongest **Eye for an Eye** among the surviving
victims (members first, then monsters; the first of equal strength) turns it
back (retail `FinishSpellEffectTargets`):

- Eye for an Eye loses one point of power.
- The spell is cast again at a power of `min(cast's power, the eye's power)`,
  with the cast's boost and no failure chance, from the victim it was aimed at
  (a group spell: from the group's leader), as strong as that victim is, on the
  caster's own side: an aimed spell on its caster; a group spell on the
  caster's group; a blast, cone or all-enemies spell on those of the caster's
  side where it went off. It flies back if it is a missile.
- A reflected spell cannot be reflected again.

### Bursts

Each victim that **Boiling Blood** killed bursts where it fell (retail
`FinishSpellEffect`): Boiling Blood Explosion is cast at the cast's
power, from the cast's owner, as strong as the owner, on whoever of the owner's
opponents stands within the explosion's radius of the corpse with a clear line
(everyone, if there is no owner; the owner's own side if the spell backfired).

### Drain-back

After **Lifesteal** has landed on all its victims, the health it took goes to
the caster's side: 70 percent of it, cut down, heals the party's member whose
health is lowest in proportion (if any is hurt); a monster caster heals
itself. After **Might to Magic**, what it took is given back as spell points
evenly to the party member whose points are lowest in proportion; a monster
caster gains half of it in budget. The amount drained from each victim is its
magnitude, cut down to the health the victim had, taken before any Guardian
Angel the victim bears takes its share. (retail `DrainTargetsLife`)

## 7. What a cast costs and trains

### Paid, whatever the roll

- **Spell points** for a learned spell: `cost * power` from its realm, the power
  actually cast. Paid on a fizzle and on a backfire.
- **A use** for an item: one charge, or one of a stack, or the thing itself if
  it is single. A potion or scroll used up is gone, but its cast still lands
  (the cast keeps its own copy). An emptied wand stays and refuses. Paid on a
  fizzle and a backfire. An item that is refused costs nothing.
- **Nothing** for a monster's own spell, but the stamina below.

### Stamina

After the cast the caster is tired:

| The cast | Stamina cost |
| --- | --- |
| a learned spell, or a monster's own spell | spell level + power |
| a thrown item | 10 |
| an instrument or gadget | `8 * spell level + 20`, even if played badly or not at all |
| any other item | below(2) + 1, doubled if Diseased |

For a member the cost, the instrument's or gadget's too, is then scaled by
load and pace as for an attack: `cost + load% * cost / 100`, with load 0, 25,
50, 100, 200 for none, light, medium, heavy, extreme, less 25 if slowed,
otherwise plus 25 if hasted ([attack](attack.md) section 6); a monster's is
taken as it stands (`FatigueCharacter`, `FatigueMonster`). At 0 stamina the caster is exhausted
([actors](actors.md)).

### Practice

A cast **affected** someone, or the caster practises nothing. Whether it did is
a rule of each spell, asked of the cast before it is rolled, so it holds
for a fizzle and a backfire too (retail `SpellAffectedTarget`):

- Heal Wounds needs a victim short of health; Heal All any member short of
  health; Stamina a victim short of stamina; Rest All, only in a fight, any
  member short of stamina; Restoration a victim with any condition or short of
  health or stamina; the cures a victim who bears what they cure (Cure Lesser
  Conditions: Irritated, Nauseated, Afraid, Asleep or Blind; Sane Mind: Insane
  or Turncoat); Resurrection a dead victim; Renewal a Drained one;
  Identify Item an item not yet identified.
- A spell laid for a time (Enchanted Blade, Missile Shield, Armorplate, Magic
  Screen outside a fight; Bless, Element Shield, Soul Shield, Ring of Fire, and
  Haste, Guardian Angel, Razor Cloak, Eye for an Eye, Superman, Body of Stone,
  Dracon Breath in a fight with hostile monsters about) needs what it would
  replace to have at most a quarter of the new duration left. The party-only
  fight spells are never "affected" outside a fight.
- A handful of spells (Light, Detect Secrets, Chameleon, Shadow Hound, X-Ray,
  Set Portal, Return to Portal, Remove Curse, and Hypnotic Lure and Sonic
  Boom outside a fight, and the four party enchantments just named outside a
  fight) share a clock each of three game minutes: they are "affected" only
  if their clock has run out, and every ask restarts it.
- Purify Air needs a cloud laid; the rest are affected unconditionally.

An affecting cast is practised as follows ([levels](levels.md) says what a use
does):

| Cast | Skill | Uses |
| --- | --- | --- |
| a learned spell | the book skill it was cast by | `(level + power + 2) / 4` |
|  | the realm skill | `level + power` |
|  | Power Cast, if available | `(level + power + 2) / 4` |
| an item | the skill that presents it | `spell level + power / 2` |
| an instrument played badly | the skill | 1 |

A save against a spell trains Iron Will (section 6). The Rust code practises
only a cast that came off and whose landing affected someone (see *Open
questions*).

### Provocation

A member's cast aimed at enemies provokes: each monster it is cast at has its
group turned hostile as the cast starts, before the roll, whatever the roll
(retail `MakeTargetGroupHostile`). A cast tried again after a fizzle provokes
no one anew. A monster, a backfire and a reflection provoke no one. A monster
under Turncoat provokes no one, nor a dead one.

## 8. Utility spells

These are not deeds on bodies. Each validates its target first, then pays and
rolls as any cast (a fizzle or backfire pays), then does its own thing. They
are cast at once, even in a fight, except Hypnotic Lure and Summon Elemental,
which in a fight are orders like any other. Each counts as an act for
practice even if it fizzles.

- **Identify Item.** The target is an item the party has (in hand, in the pool,
  or with a member), not the item casting it, and one not wholly known. It is
  refused during a fight or while the party is occupied. At auto power the
  least power that reveals it is used. On a cast that comes off: the item is
  revealed (what it is, its spell, its binding) if `3 * power`, raised by the
  boost percent, is at least the item's identify difficulty (retail
  `ApplyIdentifyAttempt`). There is no item roll.
- **Set Portal, Return to Portal.** Refused in a fight or while occupied.
  Set Portal saves where the party stands (level, position, heading) on its
  caster, replacing the one before. Return to Portal needs the caster's saved
  portal (else no target) and, on a cast that comes off, takes the party back
  there, through a level change if it is another level, stopping its motion. A
  fizzle or backfire does nothing.
- **Charm.** Cast at the one the party is talking with, while they wait for the
  party's word. On a cast that comes off the victim saves: `(level - against) *
  3 + resistance in Mental`, scaled for difficulty by the speaker's side as any
  save is (section 6), held within 5 and 95. If it fails, `amount = 7 *
  power`, raised by the boost, then cut by resistance as in section 6 (step 8);
  if the amount is above what the speaker already has been charmed by, the
  speaker is charmed by that amount, and leaves the menu as its script next
  runs. A backfire is noticed with chance `(Senses + Intelligence) / 2` of the
  victim, and the speaker says so.
- **Mindread.** Cast likewise; on a cast that comes off the victim saves as for
  Charm; if it saves, it answers with the refusal line; if not, the speaker's
  next refusal line, which advances up to the third time. A backfire is noticed
  as for Charm.
- **Hypnotic Lure.** Cast at a spot the party sees within the spell's reach
  plus eye height. It replaces any lure already standing. A lure stands for its
  rounds. Each hostile monster within the far clip of the spot, not blind, not
  asleep or worse, with a clear line from its eye to the spot, rolls a save
  (its level against the cast's strength, 5 to 95 percent, no difficulty
  scaling; an undead monster always resists): if it fails to resist, it is
  **controlled**; else it **resisted**, and does not roll for this lure again.
  A monster that does not qualify is passed over, and the rest are still
  taken (`ApplyMonsterControlToNearbyMonsters`). Outside a fight a
  controlled monster's whole group follows its choice. A controlled monster
  outside a fight, which can fight and is not blind, walks to the lure while it
  is more than 5 m from it and turns to face it; when the lure goes, so does
  the hold. (In a fight the original also takes a controlled monster's action
  from it, sending it to the lure; see *Open questions*.)
- **Summon Elemental.** Only in a fight, at a spot the party sees. The element
  is chosen by weight from Fire, Water, Air, Earth: `skill + 20` each in the
  caster's Fire, Water, Air and Earth Magic (20 each for a monster caster); Fire
  is 0 under water. The elemental is of the tier of the power (the table of the
  original's twenty-eight elemental groups, four to a power): one elemental, at
  the spot, facing the nearest monster. It is friendly to the caster's side
  unless the cast backfired. It lasts as long as the fight. With chance equal to
  the boost, it also takes its element's buff, laid as the spell would lay it:
  Fire Haste, Water Superman, Air Missile Shield, Earth Armorplate. The caster
  may have only one standing at a time.
- **Knock Knock.** Cast at a lock being picked. On a cast that comes off,
  with `level = power` raised by the boost and held at 7, it reaches `level +
  1` of the lock's tumblers chosen at random, and each moves with a chance in
  per mille by level: 344, 459, 516, 550, 573, 589, 602 for powers 1 to 7; the
  ones moved are held up by the caster's spell until the party gives up or
  opens it. A backfire drops the tumblers instead. A fizzle does nothing.
  When every tumbler is up the lock is open.
- **Divine Trap.** Cast at a trap being disarmed. The level is the power,
  boost-raised, held at 7. It records that level on the trap; what the trap
  attempt then does with it is not specified here. Divine Trap
  never backfires. While disarming, Knock Knock rolls `below(100) +
  6 * trap's difficulty` against `5 * level + 50`: below disarms; otherwise the
  trap springs (always, if it backfired).
- **A sprung trap's spell.** A trap type discharges one of fifteen spells
  (Dagger Scatter, Poison Darts, Alarm, Insanity, Whipping Rocks, Fireball,
  Freeze Flesh, Summon Elemental, Instant Death, Philosopher's Bane, Vampiric
  Vortex, Heatwave, Prismic Ray, Anti-Magic, Cerebral Hemorrhage) as a cast
  with no caster, with figure 0. `difficulty` is the trap's difficulty held to
  1 to 7; `extra = max(difficulty, the type's own grade) - grade`; the power is
  `min(7, extra + 4)`. The spell reaches `count` living members,
  `count = difficulty - below(difficulty / 2)` plus `below(extra - 3)` if
  `extra > 3`; if more members than that are alive, members are removed at
  random from the party's order, never the one who set it off. The sprung trap
  is destroyed with chance 1 in 2. (retail `g_trap_difficulty`, `g_table1`
  gives the grades and the components.)
- **Light, Detect Secrets, X-Ray, Chameleon, Shadow Hound, Magic Screen,
  Missile Shield, Armorplate, Enchanted Blade** and the others that only lay
  a spell on the party: each is an *Enchant* row (section 6) with what it
  adds in [actors](actors.md): Armorplate armour `ceil(power/2)` boosted;
  Enchanted Blade to-hit the same; Bless armour and to-hit `2` boosted;
  Armormelt `-power` armour; Magic Screen `4*power + 7` resistance in every
  realm; Element Shield `7*power` in Fire, Water, Air, Earth; Soul Shield
  `10*power` in Mental and Divine; Missile Shield `(power + 5) * 5` percent
  missile deflection; Superman `5*power` to every attribute; Haste `10*power`
  Speed and `0.1*power` of stride; Body of Stone `power` armour and `8*power`
  percent damage cut, at most 100; Chameleon `power` against being seen. Light,
  Detect Secrets and X-Ray have no number in the game's rules: they change what
  the party sees. Shadow Hound lets the party see every monster in its line of
  view at the far clip, wherever it faces. Ring of Fire, Acid Bomb, Toxic Cloud,
  Draining Cloud, Firestorm, Death Cloud **pulse** at each round's end on
  whoever they hang over (round); the party's Ring of Fire on the monsters
  within short reach of the party; the other clouds on their bearer. A pulse is
  a cast with no caster: Ring of Fire at power `power` and `against` of
  `3 * power`; the others at their own figure 0. The pulse of a cloud: Death
  Cloud, a death save at 24 (`Slay`); the others, harm by the spell's dice, and
  Draining Cloud then takes spell points (members) or budget (monsters) and
  tires, Toxic Cloud then rolls Nauseated, Poisoned and Unconscious, each its
  own save.
- **Sonic Boom and Alarm** also startle: monster groups within `30 m * power +
  50 m` of the spot take notice of the noise (retail `AlertMonsterGroupsToNoise`).
  The Rust code has no such rule (see *Open questions*).
- **Spells with no row**: Rout, Dispel Undead, Death Curse and Booze are in the
  original's spell list but have no row in the table; they cannot be cast.

## 9. Where member and monster differ

- **Power**: a member chooses it, paid for in points; a monster's comes from its
  budget, free (sections 3).
- **Failure**: a member's from skill and caster level, scaled by the pace of the
  fight; a monster's from its budget alone (section 4).
- **Strength**: a member's from total caster level, with Power Cast; a
  monster's from its level, with no boost.
- **Silence**: a member is stopped unless the spell is Alchemy and they have
  the skill; a monster, unless Alchemy and of the three kinds.
- **Items**: only members use items. Potions and food are usable by anyone, and
  in a fight a potion or food is always taken by its user.
- **What it is cast at**: a monster's own cast is aimed from the bodies it
  sees (section 5 of [monsters](monsters.md)), may not be a party spell, a point
  spell, an item spell or a lock spell, and a spell it casts at a group needs
  at least half the party to feel it.
- **Provocation, practice, Power Cast, instruments**: members only.
- **Stamina**: both pay level + power; a member's is scaled by load, a
  monster's is not.
- **Spell points**: members have them in realms; a monster has only its budget.
- **Summon Elemental**: a member's is friendly unless Turncoat (the original);
  a monster's is hostile unless the monster is on the party's side.

## 10. Open questions

Each place the code could not be explained as a rule was looked up in the
original (`Magic.cpp`, `Magic Effects.cpp`, `MonsterAI.cpp`, `PC Item.cpp`,
`Targeting.cpp`, `Spells.cpp`, `Monster.cpp`, `Combat.cpp`, `Health Stamina
Mana.cpp`). Those that were the original's own rule are folded into the
sections above and not repeated here. **Defect**: the Rust differs observably
from the original. **Missing**: the original has a rule the Rust lacks.
**Unknown**: looked at, not settled. Retail lines are in the decompilation's
`src/wiz8/local_code/` unless another directory is named; Rust lines are in
`src/game/`.

### Defects

Each entry here was checked against the decompilation a second time; those
that were real defects and could be fixed locally have been, and are folded
into the sections above.

5. **Practice is tied to the wrong thing.** Checked (`Magic.cpp:2194-2490,
   2680-2695`, `PC Item.cpp:3792-3796`): confirmed, not fixed. The original asks
   `SpellAffectedTarget` before the roll and practises if it holds, whatever
   the roll: `cast_result` is the power even for a fizzle
   (`Magic.cpp:3201`), so a fizzle trains. The Rust practises only when a cast
   came off and its landing affected a victim (`magic/casting.rs` fizzle
   branch `continue`s before `practice`; `hits/mod.rs` `affected`), so a fizzle
   trains nothing and a heal on a full party trains by what it landed on. Doing
   it as the original does needs the per-spell test of section 7 asked of the
   aimed victim before the roll, and the three-minute clocks of the handful
   of spells (new state on the party), and the remaining-duration tests of the
   laid spells.
11. **Camp.** Checked (`Magic.cpp:224-282`): unsure. `SpellUsableNow` refuses in
    camp mode every spell not usable while camped, but every place that raises
    that flag in the decompilation is the NPC dialogue or the journal opened
    from one (`NPCInteractionSubscreen.cpp:3347-3368, 4713`,
    `MainGameScreen.cpp:7041`, `MGSPortraitCombat.cpp:313`), which is where
    Charm and Mindread (usable while camped) are cast. Whether the Rust's
    camping (resting) is that mode is not settled, and the rule would refuse
    healing there. Not changed (`magic/casting.rs` `usable_now`).
14. **Auto power is generalised: what is left.** Checked (`Magic.cpp:1690-1880`):
    the list of spells, the conditions each cure counts, the average cut down,
    a monster healed at 1, indefinite conditions answered with 1, and the
    failure chance with the pace and no cap are now as the original has them;
    retail's 8 at the end of the restoring walk comes to the same as the Rust's
    7 once cut to what is affordable (at most 7). What is left: Remove Curse's
    power is not raised to the binding difficulty of the victim's worn items
    (`GetEquipmentBindingDifficulty`, `PC Item.cpp:2906`: a third, rounded up,
    of the worst identify difficulty among worn cursed items whose binding has
    not been announced), which needs the equipment and the curses' announced
    state read at that point.
16. **Drain-back.** Checked: the amount is now the magnitude cut to the victim's
    health, before Guardian Angel. The choice of member is the original's too
    (`FindPartySlotWithLowestHitPoints` compares hit-point *percent*,
    `Health Stamina Mana.cpp:1379-1397`): there was no defect there.
18. *Moved to Unknown 34.*
19. **Enemies not at war.** Checked (`Magic.cpp:3785-3790, 3830-3860`):
    confirmed, not fixed. In the original, a member's cone, blast or
    all-enemies spell cast outside a fight with no monster engaged takes the
    monsters of any disposition (neutral and friendly too, `W8_SIDE_ANY`) if
    no hostile one is found. The Rust takes only hostile ones, and a
    non-hostile monster cannot be aimed at at all (`magic/casting.rs`
    `foes_reached`). It needs a monster that is not a foe to be aimable
    outside a fight, and a blast and an all-enemies spell with a recipients
    rule that takes monsters only, not the party.

### Missing

20. **A blind caster's aim** (`CheckSpellBackfire`, `Magic.cpp:3528-3593`): described
    in section 4; not in the Rust.
22. **Set Portal refused** where the level forbids it, the party has no walkable
    ground, or the level moved this update (`Magic.cpp:2157-2160`).
23. **Party spells reach allied monsters.** A spell aimed at the party also lays on
    monsters on the party's side within the spell's radius of the party
    (`Magic.cpp:3668-3708`), and a monster may cast one. The Rust reaches the
    members only and refuses monsters the party spells (`combat/monster_spells.rs`
    `castable`).
24. **Sonic Boom and Alarm alert monsters** to the noise
    (`Magic Effects.cpp:3252` region, `AlertMonsterGroupsToNoise`). There is
    nothing in the Rust for a monster to hear.
25. **Monsters' choices of spell**: the original does not offer an enemy-group
    spell at a *group of monsters* on the other side nor a cone or blast that
    would hit a neutral monster (`SpellAreaHitsNeutralMonster`,
    `MonsterAI.cpp:1793-1823, 1862-2043`); the Rust offers the party only, and
    never vetoes (`combat/monster_spells.rs` `aims`).
26. **Rout, Dispel Undead, Death Curse, Booze** have no row (section 8).

### Unknown

27. **Missile spells' own effects.** For Acid Splash, Energy Blast, Mind Stab,
    Paralyze, Magic Missiles and Boiling Blood the original returns from its spell
    effect (`Magic Effects.cpp:3185-3193`) and lets the missile do it, with the
    missile table's chances and base magnitudes (`Combat Attack.cpp:2880-2930`).
    Whether the table rows reproduce those figures was not checked. Neither was
    the data for each spell's usable-when, reach, radius and missile flag,
    which is in the original's database.
28. **The worth of a spell for a monster** is, in the Rust, derived from the rows
    (heal the hurt, refresh the tired, afflict what is not afflicted, enchant
    what is not enchanted, cure what is borne): the original lists spells
    (`MonsterAI.cpp:1366-1790`); the common ones agree, those with special
    tests (Silence, Enchanted Blade on a monster with no attack, Sleep on one
    already asleep) were not diffed.
29. **Blast and cone sides.** In the decompiled listing a member's blast takes
    side 2 and its backfire side 1 (`Magic.cpp:3860-3866`), the reverse of what is
    played; the Rust takes foes for a normal blast. Taken here as the listing's
    artefact. A cone's range is a fixed 25 m in the original (`GetGroundTargetRange`,
    `Combat Range.cpp:1040`); the Rust uses the spell's reach.
30. **Order of victims.** The original takes party slots then monsters in the
    order they were placed; the Rust, in the order they joined the fight
    (`hits/reach.rs`). Only visible where order matters.
31. **Spell points and their recovery** (section 3) are taken from the Rust's
    comments naming `RecalculateRealmSpellPoints` and aging; the formulas were not
    re-read in the original. The ordinary item fatigue (`below(2) + 1`, doubled
    when Diseased) was: it is `CharacterActionFatigueCost`'s for using an item
    (`Health Stamina Mana.cpp:1037-1085`).
32. **Provocation.** The original provokes the aimed group as the cast starts
    (`Combat.cpp:2271-2275`) and every group the effect lists as it is processed
    (`Magic.cpp:651-653`); the Rust provokes the planned victims at the start.
33. **Knock Knock's table** and the trap-spell numbers (section 8) are the Rust's;
    only `g_trap_difficulty` was cited, not re-read.
34. **Does a member's cone take the party?** In the original a member's cone takes
    the hostile monsters in it (`CollectConeMonsterTargets` with side 1,
    `Targeting.cpp:3126-3160`); a hostile monster's takes the party and the
    monsters on the party's side. But the same routine marks the party whenever
    the party's camera lies inside the cone from the source's eye
    (`Magic.cpp:3787-3791`), which for a member (eye and camera one point) is
    always, so the listing reads as friendly fire on a member's own cone. Whether
    that is real or an artefact was not settled. The Rust takes every body but the
    caster (`magic/casting.rs` `aim_at`, `Targeting::Cone`), and the same code serves
    a monster's breath (`combat/turn.rs` `breathe`).

## How this is built

The sections above are the rules. This says how the code is to be shaped
around them; where the code differs from it, the code is wrong.

**A rule is a function of the caster, the means and the spell.** What a
cast comes from is read once: a learned spell at the power asked, an item
with what it casts and how it is used, or a monster's own casting. Each rule
above takes the caster itself, those means and the spell, and asks the means
wherever the three differ: how likely it fails, what it goes off at, what it
costs. None is handed a struct of facts copied out of the caster, the item's
record or the spell.

What an item casts, who may use it and how hard it is to identify are on the
item, written when it is made; no rule looks an item's record up.

Whom a cast reaches is read from the actors where they stand: who is in the
fight and in what order, who is of whose group, who is reached. The same
reading serves a blast, a burst where a victim fell, a spell turned back and
a cloud's pulse.

**A cast is one chain.** The observer of `Cast` reads what the rules need,
asks them in the order of this page, and makes happen what they decide:
points and charges are spent, a group provoked, the player told, and the
deed made. It holds no arithmetic and no constant.

**A deed is a thing.** What a cast does to others is an entity (`Deed`): the
rows it follows, its strength, and whom it has yet to reach. It lands on each
as it arrives, says `Landed` once all of it has, and is gone. What follows a
deed is whatever its maker put beside it, read by the maker's own observer of
`Landed`: a drain's share, the spell that may burst or be turned back, the
caster's practice, an auto recast. The code that lands a deed knows nothing
of spells, casters or fights; a breath, a cloud's pulse and a trap are deeds
too, each followed by its own maker. A cast is told as it is made, and what
its deed did as it lands.

**What a spell does is its rows.** The code that applies an effect row never
names a spell. Where one spell truly behaves alone, that is a kind of effect
or a fact in its row. A strike names whose rows it follows (a spell's, its
backfire's, its pulse's, a special attack's, a blow's); a row that says "the
spell's own" dice, rounds or realm is read against the spell as it lands.
Nothing is built from the tables beforehand.

**A utility spell is a cast like any other** up to the point where it lands;
what it then does (identify, open a portal, charm, lure, summon) is one
observer each, of the same event.
