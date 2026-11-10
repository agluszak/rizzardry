# An attack with a weapon

This page says what happens when an actor attacks with a weapon, in the game's
terms. It is the raw material for the redesign of the swing code: complete and
exact about what happens, silent about how the code is organised. Numbers and
formulas are the original's and are cited in parentheses beside them. Where the
current code does something its own comments do not say, or something that looks
like an accident of the original's program, this page describes what the code
does and lists it under *Open questions*; it does not fix it.

What an arm is, and how a member's arms are made, is on [arms](arms.md). What
armour and attacks an actor carries is on [actors](actors.md). When an actor's
moment comes is on [round](round.md). Items are on [items](items.md). This page
repeats an arm's figures only where an attack needs the exact formula.

Throughout, "roll d100" is a whole number from 1 to 100, "below(n)" is a whole
number from 0 to n-1, "dice" are `count` dice of `sides` sides plus `base`, and
every division is whole-number division unless said otherwise.

## 1. What an attack is

At its moment an actor may attack. It attacks with **one arm**, at **one
target**, in **one mode** (swing, thrust, bash, punch, kick, throw, shoot, lash,
berserk). An attack is **one or more swings**, all with that arm at that target.
How many swings there are is decided when the attack opens.

Each swing is aimed. It may be taken by a guard stepping in, or go astray onto
someone else. It reaches its target or it misses. A swing that reaches may be
stopped by armour; one that gets through lands a **blow**: damage, and perhaps
what the weapon does besides. After a swing the target may answer (Razor Cloak),
the attacker pays stamina, and what was shot or thrown is used up. The attack
goes on to its next swing while the attacker can still fight and the target
lives.

When the swings are done the whole attack is reported once, deaths are settled,
skills used are practised, and the attacker's next attack (if it has more this
round) is given its moment. Attacking holds the fight for a pause of one second.

An attack that cannot find anyone to strike is not an attack: the actor closes
in, waits, or stands (section 2).

## 2. Choosing whom and with what

### Who may be struck

The attacker looks at every living body in the fight other than itself. It may
strike a body when the attacker's stance toward it is the one it is looking for:

- It looks for **foes** (stance hostile) normally.
- It looks for **friends** (stance friendly) when it has turned on its own this
  round (insanity), see [actors](actors.md) *Side* and [round](round.md).
- The stance between two bodies is: if either is a bystander, neutral; if both
  stand the same way, friendly; if either stands neutral, neutral; if both keep
  to the same faction, neutral; otherwise they are foes. Each body's stance is
  the one it has now (Turncoat turns it about).

A body is **reached** by an arm of a given reach (touch 2 m, short 4 m, long 25 m,
extreme 50 m) as follows. Distances are between the edges of the bodies, never
less than 0.

- Two bodies both outside the party: reached when the gap is within the reach.
- A member and a body outside the party: the formation shortens touch and short
  reach. The side of the party the other body stands on (front, right, rear,
  left) is found from the party's facing, in quarters centred on the four sides.
  The member's reach against that side is cut one step for each occupied row
  between them: short becomes touch, touch becomes nothing. Rows between: none if
  the member sits in that side's row; one if that side's row is occupied; for the
  row directly across the party, one more if that row is occupied or the center
  is. Long and extreme reach are never cut. The body is reached when the gap is
  within what remains.
- Two members: reached always, except that a touch blow cannot pass the center
  rank when it is occupied and the two sit on opposite sides (front and rear, or
  left and right; the center rank itself screens nobody). A member always reaches
  itself.

A member under orders never strikes an NPC's body.

A body that is **not under orders** (a monster, or anyone not given an order this
round) must also be seen: a clear line from its eye to the other body's centre.
It also loses sight of party members at random: for each member of the party, with
chance `Stealth * 75 / 100` percent (Stealth being the member's skill points) the
member is out of its eye for this choice and cannot be chosen. A member so missed
who would otherwise have been a candidate has a one in two chance of practising
Stealth, whether or not the attacker went on to attack someone else.

### Which arm and which target

An actor has up to two **slots** (main hand and off hand; a monster has all its
attacks in the main slot) and each slot's arms. Each round the actor has a number
of attacks left for each slot. An attack is made with the slot due first, or the
other slot if the due slot has no candidate.

For each slot with attacks left, in the order due slot, then the other:

1. The candidates are every combination of an arm of that slot, a mode of that
   arm, and a body it reaches that may be struck (and, for an actor not under
   orders, is seen).
2. If there are any, that slot is used. Otherwise the other slot is tried.

(A slot with attacks left but no candidate keeps its attacks.)

Then, for an actor **under orders** (a party member):

- The target is the candidate body that is lowest by, in order: its worst
  condition (the worst condition is the greatest in the order of conditions in
  [actors](actors.md); no condition at all counts lowest, so a sound foe is struck
  before a hurt one, as in the original); then whether it is
  "in reach of the party" (those that are come first); then the smaller gap from
  the attacker. "In reach of the party" means: the body is armed (has any arm) and
  the side of the attacker's frame it stands on is the rear, or is an unoccupied
  side while the center rank is occupied. Ties keep the fight's order, then the
  order of the arms.
- The arm is the first arm of the chosen slot that reaches that target.
- The mode is Berserk if the member's order is Berserk. Otherwise one of the
  arm's modes is chosen by drawing `w` below 9 and counting `w` modes along the
  arm's modes (in the order swing, thrust, bash, berserk, throw, punch, kick,
  lash, shoot), going round from the start if needed. One mode: that mode. Two:
  the first 5/9 of the time. Three: a third each. (retail `ChooseAttackMode`)

For an actor **not under orders**:

- Every candidate (arm, mode, target) has an equal chance. A wary monster that
  chose to hold back this moment has one more equal chance, to wait instead
  (whether it holds back is decided elsewhere, see [round](round.md)). The pick is
  below(candidates + waits); if it falls in the waits, the actor waits.
- One that already made an attack this round, or moved, does not decide again
  and does not hold back; it simply strikes.

(retail `ChooseMonsterTarget`, `BuildMonsterActionQueue`)

If a slot is chosen, the actor's attacks left in that slot drop by one, and that
slot becomes the slot due.

### When nobody can be struck

- An actor that has a stride and can walk, and has not moved this round, **closes
  in**:
  - If its stance toward the party is friendly (a turncoat monster), it goes
    toward the nearest foe it could strike (squared distance between feet),
    provided it can move (walking speed above 0). Hurt (health at 95 percent of
    its maximum or less) it **charges**.
  - If hostile, it goes toward the party, provided its longest arm does not
    already reach the party within 0.3 m (gap greater than reach less 0.3 m) and
    it can move. The first living member in the party's order stands for the party.
  - If neutral, it does nothing.
  - How far it goes: its stride, times 1.5 if it charges; plus its stride times
    its haste share (a tenth per power of Haste) when hasted, otherwise halved if
    slowed. (retail `SetUpMonsterTurn`)
  - It has then moved; its next moment is twice the current moment, no later than
    100. A notice says it advances (or charges).
  - A member does not close in (it has no stride), nor does one that has moved.
- A wary actor that chose to wait **waits**: it is done for the round and has
  its breath given back at the round's end.
- Anything else **stands**: done for the round.

An actor that closed in strikes when its next moment comes, without deciding
again.

## 3. Before the first swing

When the attack opens it is announced and, before that, two things settle how
the target stands toward the blow.

**Catching the target unaware.** Only a blow with touch or short reach can do it.
Find the side of the target that the attacker is on (for a monster target, from
the monster's own facing; for a member target, from the party's facing, as in
section 2) and the side the target faces (a monster: its front; a member: the side
of their row, the center facing front, unless they turned). If these are the same
side, nothing happens: the target sees it coming.

Otherwise, a **member** who is defending (their order is Defend), alive, with stamina
above 0, not worse than Turncoat (nothing worse than Turncoat, so not webbed,
asleep, paralyzed, unconscious), and not blind, **may notice**:

- The first time this round that a member tries, they notice for certain.
- Each later time, they notice with chance `Senses - 25 * (tries so far)` percent.
  The count of tries is kept for the round and rises every time, whether they notice
  or not. If this figure is negative the member notices for certain (the original
  compares it as an unsigned number: retail `CharacterNoticesAttacker`).

A member who notices turns to face the attacker's side and is not caught.
Anyone else (a monster, an undefending member, one who fails to notice) is caught
**unaware** if the attacker is exactly on the side across from the way the target
faces (the rear of a facing front, and so on; the center has no opposite). A
target not caught is not caught for this attack by this rule.

**Natural attacks.** An arm may **ambush**: a member's sword or dagger arm of
touch reach when they have the Backstab trait, a monster of the rogue kind's
attack of touch reach. Every blow of such an arm is a *natural* attack. It counts
as surprise (below) whether or not the target faces the attacker or the attacker
was hidden: the code asks nothing about hiding.

**Provocation.** A party member (not under Turncoat) who attacks provokes the
target: its group turns hostile as the attack starts, before the blows land,
whatever the blows then do.

**Announcement.** The player is told the attacker, the target, the mode (a
monster: its own word for the attack, and whether "at" follows), the number of
swings, and whether it is a surprise. Deaths and damage are told after the swings.

## 4. A swing

### How many swings

An attack makes `below(swings) + 1` swings, where `swings` is the most swings of
the arm, then:

- add the weapon's extra swings (it may be negative), and make at least 1;
- if this is the main slot, the actor is a Samurai whose Lightning Strike was
  readied this round (see section 5), the arm is a sword, and the actor is not
  slowed: add 3 if `below(11) + Samurai level < 20`, otherwise 4. The readied
  Strike is used up by the first attack of the main slot, sword or not, slowed or
  not.

A swing is not made if the attacker can no longer fight (alive, and no condition
as bad as asleep) or the target is dead. The rest of the swings are then lost; no
new target is chosen.

### An arm's figures

These come from the actor ([arms](arms.md) for members; the species for monsters).

A member's arm:

- **Skill score** of a hand: `(combat + weapon*2) * 2 / 3`, where combat is Ranged
  Combat for a ranged hand and Close Combat otherwise, and weapon is the skill
  the weapon is fought with (Martial Arts for a fist). If the other hand holds a
  weapon, add half that weapon's skill and the divisor is 25, otherwise it is 20.
  If both hands hold weapons and both strike (dual wielding), add the Dual Weapons
  skill and add 10 to the divisor. The score is `score * 10 / divisor`.
  (retail `CalcAttacks`)
- **Dual penalty** of a hand: 0 unless dual wielding; then `-10` for the main hand,
  `-20` for the off hand, `- (100 - Dual Weapons) / 4`.
- **Rating** of a hand: `(dual penalty + Dexterity/2 + skill score*2 + physical
  levels) / 3 + 60`; none (0) when the hand cannot strike.
- **To hit** of a hand (the sheet's number): the weapon's own plus the missile's;
  plus `(Strength-50)/(divisor*10)` above 50 or minus `(50-Strength)/(divisor*10)`
  below; plus `(Dexterity-50)/10`; minus `(30-Senses)/10` if Senses is below 30, plus
  `(Senses-70)/10` if above 70; plus the load's penalty (negative; one and a half
  times in the off hand; halved for modern weapons); plus `Eagle Eye/20 + 1` for a
  ranged hand if the skill is available, or `Power Strike/20 + 1` for a close
  hand if available. The strength divisor is 1 for full strength, 2 for a weapon
  strength helps by half, and strength counts for nothing for guns and the like;
  in the off hand when Strength is above 50 the divisor is doubled.
- **Arm rating** used in the attack is `rating + (to hit + spell to-hit bonus) * 5`.
  (Bless and Enchanted Blade give the bonus.)
- **Press** (how well it presses past a guard, or guards against a blow):
  `Speed + main-hand to hit * 5 + main-hand rating + spell to-hit bonus * 5`.
  (retail `GetCharacterProtectionScore`)
- **Attacks a round** in a hand: `score = ((Speed + Dexterity)/2 + dual penalty +
  physical levels + load's attack penalty (halved for modern weapons) + skill
  score) / 3`. Main hand: 1, plus 1 above 49, plus 1 above 99. Off hand: 1, plus 1
  above 74.
- **Swings** (most) in a hand: 1 if the main hand holds a single-swing weapon
  (rocket launcher). Otherwise `score = (Speed + hand's initiative*10 + dual
  penalty + physical levels + load's attack penalty (halved for modern) + skill
  score) / 3`; swings are 1, plus 1 above 66, plus 1 above 99.
- **Initiative** of a hand: the weapon's; a fist's is `skill score / 10`.
- **Damage percent** of a hand: a launcher's own, plus strength: `(Strength*2 -
  100)/divisor` above 50, `-(50-Strength)/divisor` below; none for guns.
- **Dice**: a weapon's own, or a launcher's missile's; a fist's by Martial Arts /
  11 from a table (`1d2`, `1d3`, `2d2`, `2d3`, `2d4`, `3d3`, `3d3+1`, `3d3+2`, `3d5`,
  `4d4`, `4d4+2`, `4d4+4`), main hand two more on the base. A punch rolls the off
  hand's dice: those of whatever it holds, as the item's record gives them (a shield
  has none, so a punch past a shield does the minimum), or an empty off hand's fist
  dice; a kick rolls the main hand's fist dice.
- **Modes**: the weapon's, or a fist's punch (and kick if Martial Arts is above 4).
- **Effects**: a weapon's with its missile's added (each chance added, to 100; the
  strengths added); a fist has a Knock Out chance of `(Strength-50)/5` percent when
  Strength is above 49.
- **Slays**: the monster kinds the weapon and the missile slay.
- A **modern weapon** (modern weapons skill) feels fatigue half as much in its aim.
  A weapon not of that skill feels fatigue in its penetration (see below).
- An arm is **sure** (a thrown weapon of a Ninja, not a sling) if the member has
  Thrown Criticals and the weapon is a thrown ranged weapon.
- The **critical skill** of an arm: Critical Strike for any close arm; for a ranged
  bow, modern weapon or sling, Ranged Combat if the member has Ranged Criticals;
  for other thrown weapons Critical Strike if the member has Thrown Criticals;
  otherwise none.

A monster's arm, from its species:

- rating is the attack's score plus 5 times the spell to-hit bonus; damage percent
  0; initiative 0 (the monster's own initiative comes with it); attacks a round
  and swings (at least 1) from the species; effects from the attack; no slays;
  ambushes if a rogue's touch attack; modern and fatigue-in-penetration both when
  the attack's weapon is a modern one (the original's list of eighteen weapon
  names, from the Musket to the Plasma Bolt); it costs no stamina; it never uses up anything.
- press: `Speed + first attack's score + spell to-hit bonus * 5`.

### The roll to hit

For the swing, let:

- **Attacker's fatigue** (retail `FatigueArmorPenalty`) by the share of stamina
  spent: under 50: 0; 50 to 69: 5; 70 to 84: 10; 85 to 94: 20; 95 or more: 40.
  Half of it (rounded down) for a modern weapon (a monster: an attack that tires
  little).
- **Agility** of an actor: Senses + Speed + Dexterity + Intelligence.
- **Mode's to-hit**: swing, bash, shoot 0; thrust, kick -10; berserk -30; throw -5;
  punch +5; lash +10. (retail `AttackModeMod`)
- **Surprise**: +10 if the blow is unaware, +10 if it is natural. Both can hold.
- **Target's armour against the aim**: the target's total armour, plus 2 (members;
  the bracing figure) if the target is defending, plus the target's *guard*:
  - fatigue/10 of the target, for a target whose fatigue tells on armour
    (monsters; not members);
  - minus, when the blow is a surprise, the armour its surprise strips
    (for a monster, twice its vulnerability from behind; for a member, what Speed
    and a shield give, and more if defending, see [arms](arms.md)), plus the
    defending member's extra strip if defending;
  - minus 4 if the target is on the move and the mode is not a throw, lash or
    shot. The target is on the move if it is a monster that is walking this
    round, or a member while the party is running.
- **Difference of agility**: `(attacker agility - target agility) / 10`.

Then:

```
base  = arm rating + mode's to-hit + surprise - fatigue
guard = base - 5 * target's armour against the aim + difference of agility
```

If the attacker is **blind** and the figure is above 10, the amount above 10 is
lost: the figure becomes 10; or, for one who fights blind (a member with the trait,
a Monk monster), the amount above 10 is scaled by the attacker's level and added to
10: `10 + (figure - 10) * (level*2 + 60) / 100` (level over 20 counts as 100), cut
down to a whole number.

The result is scaled for difficulty by the attacker's side (below). That is the
**aim**.

**Difficulty.** On Normal, or for a neutral, a figure is unchanged. On Novice a
hostile's figure is multiplied by 0.6, a friendly's by 1.4. On Expert a hostile's
by 1.4, a friendly's by 0.6. (The same table applies wherever "scaled for
difficulty" is said.) (retail `ScaleValueForCharacterDifficulty`,
`ScaleValueForMonsterDifficulty`.)

The aim and the penetration are scaled by the attacker's side; the target's saves
(section 5) by the target's side.

### A close swing (reach under long)

1. Roll d100.
2. Find the **bystanders**: every living body in the fight other than the target,
   that is the attacker itself or is reached by this arm from the attacker (the
   attacker, friends and foes alike).
3. The **fumble chance** is worked from the **fumble odds**, which are the base
   figure with the blind rule and difficulty applied, but not the target's
   armour or agility:
   - 0 if the figure is 100 or more;
   - otherwise `chance = (100 - figure)^3 / 100000 + 0.5`, cut down to a whole
     number, and the fumble chance is `bystanders * chance * 10 / 100`, at most
     100.
4. If the roll is **5 or less** the swing reaches and **gets through armour** (no
   fumble, no penetration needed).
5. Otherwise, if there are bystanders and the roll is above `100 - fumble chance`
   the swing **fumbles** (below).
6. Otherwise, if the roll is **above 95** the swing misses.
7. Otherwise the swing reaches the target; it lands if the aim is at least the roll.

A **fumble** lands on one bystander picked at random (equal chances). The player
is told. A fresh d100 is rolled for the penetration, the swing counts as having
reached (aim is not asked), but is not sure through armour. The attack's surprise
is gone for the rest of the attack. The blow's damage is limited, and it is told
on its own, not counted in the attack's hits or damage (section 5).

### A missile swing (reach long or more)

There is no fumble and no automatic hit or miss. The shot is aimed at the target's
centre from the shooter's eye. Its aim point is scattered across the line of
flight:

- `D` is the distance in metres from eye to centre. If `D*500 < 1` the aim point is
  the centre exactly.
- The direction of the error across the line is chosen by an angle `below(360)`
  degrees.
- Seen with a sighted shooter: `factor = 0.08`, `widest = 0.2`. A **blind** shooter:
  `factor = min(1/D, 1.75)`, `widest = 1.75`. `closest = factor * D`.
- `spread = floor(D)`; `effective = (100 - spread) * aim / 100`.
- Base offset: `widest * D` if `effective <= 0`; `closest` if `effective >= 300`;
  otherwise `closest + (widest - factor) * (300 - effective)/300 * D`.
- Roll `r` d100. If `min(effective, 95) >= r` the offset is `r / effective *
  closest`. Otherwise, if `effective >= 1` or `r + effective >= 1`, the offset is
  `(base offset - closest) * w + closest`, where `w = (r - effective)/r` for
  `effective >= 1`, and `w = (100 - (r + effective)) * 0.01` below 1. (retail
  `ScatterMissileAimPoint`)
- The shot strikes when the aim point is within the target's radius horizontally
  and within its half height vertically of its centre.

A shot that misses simply misses. One that strikes rolls a fresh d100 for the
penetration, is not subject to the aim comparison, and gets through armour
whenever the arm is *sure* (a Ninja's throw). The shot does not meet other bodies
on the way (the original can hit whatever is in the line).

### Guards intercepting

Only a **monster** can be guarded: a monster may spend its moment guarding a fellow
for the round. Each guard's number of interceptions this round is its main-slot
attacks a round.

When a swing has reached (including a fumble or a shot that strikes), before the
location is found, each guard of the swing's victim steps in if:

- it is alive and its worst condition is below Blind;
- its first arm's reach in metres is at least the gap between it and its ward
  (formation is not asked);
- it has interceptions left;
- `below(100) < clamp(guard's press - attacker's press + 50, 5, 95)`, where the
  press of each is its press less its fatigue (halved for one who tires little),
  and the attacker's press is further raised by 10 if the blow is unaware and 10
  if natural.

If any step in, one is picked at random. It takes the blow in its ward's place:
its interceptions used rise by one, it loses `below(3) + 2` stamina, the blow's
surprise is gone for the rest of the attack, and the player is told. The guard is
now the victim for everything that follows (location, penetration, damage); the
aim already made against the ward stays.

### Where on the body

The blow lands at a location of the victim, picked by one d100 against cumulative
percentages:

- On a **monster**: its species' chances for its seven locations; anything past the
  total lands at location 3 (its body).
- On a **member**: if the attacker is a monster, its species' chances for the five
  parts (head, body, legs, hands, feet); otherwise (a member struck by a member)
  15, 40, 30, 10, 5 percent for head, body, legs, hands, feet. Anything past the
  total lands on the body.

The location sets which armour the blow meets (below).

### Penetration

A swing that has reached (the aim is at least the roll, or it was sure, or it was a
fumble or a shot that struck) must now get through armour. It does so when it is
sure through armour (a roll of 5 or less in close combat, a Ninja's throw), or
when:

```
penetration >= roll
```

where `roll` is the same d100 as for the aim in a close swing, and the fresh d100
for a fumble or a shot, and

```
penetration = scaled( mode's penetration
                      - attacker's fatigue (only if it tells on penetration)
                      - 5 * victim's armour at the location
                      + difference of agility )
```

- **Mode's penetration** (of the mode that pierces: a launcher's missile mode,
  drawn from the missile's modes by the same mode draw as above, otherwise the
  swing's mode): swing, kick 0; thrust, shoot +10; bash, punch -5; berserk -30;
  throw +5; lash -10. (retail `AttackModeMod`)
- **Fatigue** counts here for a member unless the weapon is a modern weapon (full
  amount, not halved); for a monster only for an attack that tires little.
- **Victim's armour at the location**: its armour at that location, plus 2 if
  defending, minus what the victim has against that mode (monsters only), plus
  **twice** the target's guard as defined above (fatigue/10, minus the surprise
  strip, minus 4 on the move). The surprise now is the attack's surprise as it is
  after any interception or fumble.
- The difference of agility is with the actual victim.
- Scaled for difficulty by the attacker's side. The blind rule is not applied.

**Missile Shield.** If the mode is shoot or throw and the victim's Missile Shield
is above 0, the blow is turned aside with chance equal to the shield in percent
(`below(100) < shield`). Then no damage and no weapon effect lands, and the victim
does not practise Iron Skin.

If the swing does not get through, it does no damage. It still counts as a hit
(section 7) and still draws a riposte.

## 5. A blow that lands

### Damage

Count the dice rolled for the blow, `dice count`:

1. Start at 2 if the blow is unaware or the victim is on the move, and the mode is
   not a throw, lash or shot (a **close** mode); otherwise 1.
2. If the blow is natural and the mode is close, add the **backstab** dice (below).
3. If the mode is berserk, add 1.
4. If the count is above 1, the victim is defending and the blow is not unaware,
   subtract 1.
5. Add 1 for each of the arm's slays (the weapon's, the missile's) that equals the
   victim's creature kind. (A member has no kind.)

Roll the arm's dice that many times, summing every full roll (the base is added to
each roll, so it is counted `dice count` times). The arm's dice are the punch dice
for a punch, the kick dice for a kick, otherwise the arm's. (Retail
`ResolveCharacterAttackDamage`.)

```
damage = max(1, ((100 + damage percent) * rolled + 50) / 100)
```

Then the victim's **damage reduction** (percent, from its armour; spells add, to
100): if it is not 0, `damage = ((100 - reduction) * damage + 50) / 100`, which
may be 0.

A blow that went astray (a fumble) and does damage is limited to a fifth of the
victim's maximum health (at least 1). (retail `CapAttackDamageByTargetHealth`)

**Backstab dice.** The attacker's backstab chance is, for a member, `100` scaled
by profession level, for a monster, `100` scaled by its level (60 percent at level
0, 2 points more each level, full above level 20). Roll below(100) = `r`. Extra
dice: 1 if `r + 15 < chance`, plus 1 more if `r + 75 < chance`, plus 1 more if
`r + 95 < chance`. (retail `RollBackstabExtraDice`)

### Critical and knock out (members only, not a fumble)

Before the blow lands, when it has got through armour:

- If the arm has a critical skill, the blow's **Kill** chance (a percentage the
  weapon may already have) rises by `skill / 25`, plus 1 more with chance `(skill
  mod 25)/25` (one more when `below(25) < skill mod 25`, and the skill is not a
  multiple of 25). The Kill chance is at most 100.
- If the member has the Knock Out trait, the **Knock Out** chance rises by `5`
  scaled by profession level (3 at level 0 up to 5, whole number), at most 100.

(retail `ResolveCharacterAttack`)

### Landing the blow

A blow that did more than 0 damage lands as a **hit** on its victim:

1. Damage is dealt as harm: Guardian Angel takes what it can, two thirds of the
   rest tires the victim, health falls ([actors](actors.md) *Being hurt*). If the
   victim was asleep and survived, it wakes with chance `Senses/2 + 50` percent.
2. Then the weapon's effects, in this order, each one only if the blow's chance for
   it is above 0:

| Effect | Condition or result | Realm resisted | Save base | Rounds / amount | On |
| --- | --- | --- | --- | --- | --- |
| Sleep | Asleep | Air | 20 | rounds | all |
| Paralyze | Paralyzed | Water | 18 | rounds | all |
| Poison | Poisoned, bite strength = the blow's effect strength | Water | 6 | `(3*c)d2 + strength + 1` | all |
| Hex | Hexed | Divine | 8 | rounds | all |
| Disease | Diseased, lasts until lifted | Divine | 2 | 9999 | all |
| Kill | the victim dies outright | Divine | 24 | 9999 | all |
| Knock Out | Unconscious | Earth | 22 | rounds | all |
| Blind | Blind | Fire | 12 | rounds | all |
| Frighten | Afraid | Mental | 5 | rounds | all |
| Possess | Turncoat | Divine | 14 | rounds | members, and only from a monster's blow |
| Drain Health | a member's maximum health drained | Earth | 0 | `c d2 + 1` | members |
| Drain Stamina | stamina lost | Water | 0 | `(2c)d5 + 3c + 1` | all |
| Drain Spell Points | spell points lost | Divine | 0 | `c d6 + c + 1` | members |
| Nauseate | Nauseated | Air | 4 | rounds | all |
| Insane | Insane | Mental | 10 | rounds | all |

Here `c = effect power / 3 + 1`. The effect power is `level + level` for a
level of 14 or less, and `level + 15` above, the level being the attacker's
(a member's is the sum of profession levels). It is both the strength of the
effects and what the victim's level is weighed against. (retail effect power
level)

Each effect is settled in turn:

- It is skipped if the victim is dead (every effect needs a living victim), so
  nothing lands on a victim the blow itself killed.
- It lands only with its chance: `below(100) < chance` (the chance is the blow's
  chance for that effect; a Kill or Knock Out chance includes what training added).
- It does nothing to a victim immune to that condition (monsters of some kinds:
  undead, android, plant, see `defences`), and to a member's wards.
- The **save**: the victim saves with chance `(victim's level - effect power) * 3 +
  save base + victim's resistance in the realm`, scaled for difficulty by the
  victim's side and held within 5 to 95 percent: `below(100) < chance`. A member who
  saves practises Iron Will twice.
- The **amount** (rounds, bite or loss) is rolled as in the table: with chance 1 in
  4 it is just 2; otherwise `dice + 1`. A rounds roll is `c d2 + 1 + 1`, held
  between 1 and 12. A rounds figure of 1 becomes 2. The amount is then cut by the
  victim's resistance: if `resistance + (victim's level - effect power) * 3` is
  above 0, add 1 and take a random share (the sum of `below(c - c/2)`, `c/2` and
  `below(c/2)`, at most 100 percent) off the amount. An amount cut to nothing means
  the effect is resisted.
- Possess has no effect on a victim whose stance is neutral.
- A save, a resistance, a ward: each makes the effect "resisted" and the player is
  told so.

Effects are told after the whole attack, with what the attack did besides damage
(section 7).

### Lightning Strike

A Samurai (the Lightning Strike trait) whose order for the round is Attack, who
wields a sword in the main hand, and who is not surprised in the round's opening,
readies a Lightning Strike as the round begins with chance `scale(12)` percent by
profession level: `below(100) <` (7.2 plus 0.24 per level, whole number; 12 from
level 21). It adds swings as described in section 4. (retail `BeginCombatExecution`)

### Berserk

A Berserk order (needing the trait, an arm, and a main hand of short reach or less)
makes every attack of the round in berserk mode: -30 to the aim, -30 to the
penetration, one more die, and double the stamina per swing. The attack is still
against foes. (The insane turn on their friends in an ordinary mode instead.)

## 6. What an attack costs and trains

### Stamina

After each swing, hit or miss, a member whose arm tires pays stamina. A monster
pays none.

```
cost = punch:  2 + below(3)
       kick:   3 + below(3)
       weapon without a weight: 3 + below(4)
       weapon of weight w:      band = w/40 + 1;  below(band) + 1 + band
cost = cost * 2 if berserk * 2 if diseased
```

Then the load scales it: `cost + percent * cost / 100`, where `percent` is the
load's: none 0, light 25, medium 50, heavy 100, extreme 200; minus 25 if slowed,
otherwise plus 25 if hasted (slowed wins). (retail `CharacterActionFatigueCost`,
`FatigueCharacter`)

The stamina is lost (to nothing at most); running out of it exhausts the member.

### What is used up

Only a member's missile weapons use anything. After each swing:

- A weapon that casts its spell, or a thrown missile that is kept, uses nothing.
- A launcher uses one of the missiles in the other hand every swing.
- Anything else ranged (a thrown weapon, a gun) uses one of itself every swing: a
  charge, or one of a stack, or the thing.
- When what the weapon counts (charges, else units) is used up by the swings of
  this attack, the slot has no attacks left for the round and the attack ends at
  once.

The last missile is spent by the swing that uses it; that swing is resolved first.

### Skills practised

Practising a skill a member can train (skill available to their profession) is a
use of it; what a use does is on the skills page. After a blow that did damage and
is not a fumble the attacker practises:

- close combat: Critical Strike, Power Strike;
- ranged: Eagle Eye;
- ranged, with the Thrown Criticals trait: Eagle Eye and Critical Strike.

The victim, if a member, practises Iron Skin whenever a blow got through armour,
whether or not it then did damage. A member who saves against an effect practises
Iron Will (twice). A member missed by a wild attacker's eyes may practise Stealth
(section 2). Nothing else is practised by an attack.

## 7. After the attack

### Razor Cloak

After every swing in which the swing reached its victim (the aim comparison
passed, or sure, or a fumble, or a shot that struck, whether or not it got through
armour) and the arm's reach is under long, the victim answers if it wears Razor
Cloak:

- The cloak loses a point of power and is lifted at none. The answer uses the power
  it had before the point was spent.
- Damage is the spell's dice rolled as `count*power` dice, no boost, less the
  attacker's own damage reduction (`((100 - reduction)*d + 50)/100`).
- If it is more than 0 it lands on the attacker as harm (no weapon effects).
- It is told on its own: "victim hits attacker for N".

The victim is the one the blow landed on (a guard that took it, or the bystander
a fumble struck).

### What is reported

In this order:

1. The attack is announced (section 3).
2. As they happen: a guard stepping in; a fumble. A fumble's blow, and a Razor
   Cloak answer, are each told at once as a single hit with its damage, followed by
   any other effects it had.
3. When the swings are done: the attack's result, `hits` and `damage`, the target
   and the attacker, and then, if any, everything its blows did besides damage.
4. Deaths are settled and told; health lost is told for display.

`hits` counts swings that reached a victim other than a fumble's, whether or not
they got through armour or did damage. `damage` counts the health lost, after
Guardian Angel, by the blows landed on the intended or intercepting victim.

### The next attack

The actor has attacks left for each slot. After this attack:

- The next slot is the other slot if it has attacks left, unless it has fewer than
  the current slot and less initiative; otherwise the current slot.
- If the next slot has no attacks left, the actor is done for the round.
- When the next slot is the current slot, or the other slot has no more attacks
  than the current, the next moment is
  `now + (100 - now) / (attacks left in that slot + 1)`. Otherwise it is `now`.
- Changing hands adds the current slot's initiative less the new slot's. The
  moment is held between `now` and 100. (retail `ComputeCharacterActionPhase`)

Here `now` is the moment of the round in which this attack was made. A monster has
only its main slot.

The first moment of the round for an actor that attacks is
`90 - clamp(below(4) + actor initiative + opening slot's initiative, -10, 89)`,
so 1 (first) to 100. The opening slot is the one with the most attacks, then the
greater initiative, and the main hand on a tie. (retail `AssignCombatPhases`)

An order is accepted when the round begins (see [round](round.md)); a member who
no longer holds a weapon they were accepted with loses that attack.

## 8. Where member and monster differ

- **Arm figures**: a member's from their sheet and gear (section 4); a monster's from
  its species (rating = score + 5 * spell bonus; swings at least 1; attacks a round).
  Press differs likewise.
- **Choosing the target**: a member under orders by condition, "in reach", gap; a
  monster (or anyone not ordered) at random, with a wait among the choices, and
  needing sight. Only the unordered lose party members to Stealth.
- **Choosing the mode**: a member by the mode draw (or berserk); a monster by an equal
  chance for each (arm, mode, target).
- **Ordered members never strike NPC bodies.**
- **Closing in**: only a body with a stride (a monster) that can walk.
- **Natural attacks**: a member with Backstab and a touch-reach sword or dagger; a
  monster of the rogue kind with a touch attack. Backstab chance scales by the
  member's profession level or the monster's level.
- **Catching unaware, noticing**: only a defending member notices; a monster does
  not (the original lets a waiting monster notice; see *Open questions*). A monster target is caught unaware only from its rear.
- **Provocation**: only a member's attack provokes.
- **Armour**: a monster's armour is "10 less the species value" by place with a
  separate figure per mode and a table of hit locations, its guard rises with its
  fatigue, and it strips by twice its vulnerability from behind; a member's armour
  comes from Speed, race, load, shield and worn gear, bracing when defending, and
  the surprise strip by Speed and shield. A monster is never braced.
- **Fatigue**: a modern weapon (members) or an attack that tires little (monsters)
  halves fatigue in the aim; fatigue counts in the penetration for a member's
  non-modern weapons but for a monster only for the attacks that tire little.
- **On the move**: a monster that is walking; a member while the party runs.
- **Where blows land on a member**: by the monster's species table; by 15/40/30/10/5 for
  a member's.
- **Critical, knock out, Lightning Strike, Knock Out trait, skills practised,
  stamina, ammunition**: members only.
- **Weapon effects**: the Possess effect only from a monster's blow; a monster has no
  critical skill.
- **Interception**: only monsters can be guarded or guard; a member's blow is
  intercepted only because its ward is a monster (the original lets members protect
  too; see *Open questions*).
- **Blind fighting**: a member with the Effective While Blind trait (scaled by their
  profession level); a Monk monster (by its level).
- **Effect power**: the attacker's level (a member's the sum of profession levels).
- **Damage reduction**: a member's from their sheet and spells; a monster's from its
  species and spells.
- **Difficulty**: by the side of the one it applies to, never by "member or monster".
- **Slays and creature kind**: a monster has a kind; a member has none and is never
  slain-against.

## 9. Open questions

Each place the page once could not explain was checked against the original
(`Combat Attack.cpp`, `Targeting.cpp`, `Combat.cpp`, `FormationAndFacing.cpp`).
Those that were the original's own rule are now stated as rules above; one was a
defect and is fixed (the punch dice). Two remain, both missing behaviour of the
original, not understood rules.

1. **A waiting monster may notice an attack** (`catch_unaware`, `swings.rs`). In the
   original (`StartCharacterAttack` for a member's blow, `AnnounceMonsterAttack`
   for a monster's), a monster target that is not facing the attacker, is alive,
   has stamina, is not worse than Turncoat and not blind, and whose action this
   round is to wait, notices exactly as a member does: certain the first time
   in the round, then by `Senses - 25 * tries` (as an unsigned number), keeping its
   own count of tries; a monster that notices turns to face the attacker. Only
   otherwise is it caught unaware, from its rear. Here a monster never notices.
   Defect, not fixed: it needs each monster's action for the round to be known
   before its moment (the original decides it as the round opens; here a monster
   decides at its moment), a count of tries on monsters, and a monster turning.
2. **Members that protect** (`turn.rs`, `Guarding`, `Guards`; `intercept`). In the
   original a member whose order is Protect (retail `ACTION_PROTECT`) takes blows
   meant for the one it protects, a member or a monster, by the same protection
   score against the blow's press, up to its main hand's attacks a round
   (`ResolveGuardianInterception`). Here only monsters guard, and only monsters
   are guarded; there is no Protect order. Defect (missing), not fixed: it needs
   the order itself and members to be guarded.

## 10. How this is built

The sections above are the rules. This section says how the code is to be
shaped around them; where the code differs from it, the code is wrong.

**A rule is a function of the facts it is about.** Each numbered rule above is
one function that takes plain values and returns a plain value. None of them
takes a query, a world, a catalog or a log, and none of them changes anything.

| Rule | Takes | Gives |
| --- | --- | --- |
| who may be struck, and with what (2) | for each candidate: how the attacker stands toward it, which arms reach it, whether it is seen, its worst condition, whether it is in reach of the party, the gap | the arm and target, or what to do instead (close in on whom, wait, stand) |
| catching unaware (3) | the reach, where the attacker stands about the target, what the target is doing, its Senses and how often it has tried this round | whether the blow is unaware, whether it is natural, whether the target turned |
| the roll to hit (4) | the arm's rating, the mode, the attacker's fatigue and blindness, the surprise, the target's armour and what it is doing, both agilities, the difficulty | the chance |
| the swing's fate (4) | the chance, the bystanders in reach | reaches (and whether surely), misses, or fumbles onto which bystander |
| who steps in (4) | the guards in the order they took it up, each one's reach and blows left, the attacker's press | the body the blow lands on |
| getting through armour (4) | the arm's rating, the mode it pierces by, fatigue, armour at the place struck, both agilities, the difficulty | the chance |
| the blow (5) | the arm's dice and percent, slaying, backstab, the kill skill, the weapon's effects | the damage and what else it does |
| what a swing costs (6) | the mode, the weapon's weight, berserk, disease, load, slowed, hasted | the stamina |
| the next attack's moment (7) | attacks left with each hand, each hand's initiative, the moment now | which hand and when, or none |

The facts each takes are a small struct named for the rule, built in one
place. Every number on this page is a test of one of these functions, with
the expected value worked out from the page, not from running the code.

**The attack itself is one piece that only arranges.** The observer of
`Strike` reads what the rules need about the attacker, the target and those
around them, calls the rules in the order of this page, and makes happen what
they decide: the blow lands at once, since the next swing and what is told
of the attack depend on what it did; what was shot is spent (`Spend`), skills are practised (`Practise`), a group is provoked
(`Provocation`), the player is told (`CombatNews`). It holds no rule of its
own: no arithmetic, no constant.

**What is read of an actor is one view.** The attacker and the target are read
through the same view of an actor (health and what ails it, attributes, level,
`Attacks`, `Armour`, side, place), whichever of member and monster they are.
The places section 8 lists where the two differ are differences in the facts
refit wrote, never a branch in a rule here.

**What goes.** `open`, `swing`, `intercept`, `catch_unaware`, `land_blow`,
`riposte`, `next_attack` as functions that take the world's queries and a
mutable attack and do several rules each; the `Attack` value threaded mutably
through them; arithmetic and constants inside the observer.
