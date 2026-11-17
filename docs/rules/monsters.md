# What a monster does

This page says how a monster decides what to do, in and out of a fight, in the
game's terms. It is the raw material for the redesign of the monster code:
complete and exact about what a monster does and when, silent about how the
code is organised. Numbers and formulas are the original's and are cited in
parentheses beside them. Where the original was looked up and the current code
does something else, the text says what the code does and the *Open questions*
section says what the original does; the code is not fixed here.

What an actor is, what ails it and which side it is on is on
[actors](actors.md) (*Conditions*, *Side*, *Place*). When an actor's moment
comes and how a round runs is on [round](round.md). The weapon attack a monster
makes, how it picks its target and how it closes in when it cannot strike is on
[attack](attack.md) section 2; this page does not repeat it. The spell it casts
is on [casting](casting.md); this page says which spell and at whom.

Throughout, "roll d100" is a whole number from 1 to 100, "below(n)" is a whole
number from 0 to n-1, and every division is whole-number division unless said
otherwise. Distances are between the edges of bodies, never less than 0. The
reaches are touch 2 m, short 4 m, long 25 m, extreme 50 m. A monster's own
figures (temper, special attack, spells, stride, sight bonus, group size,
faction, hostility radius) are its species record; they are read when it is
made.

## 1. What a monster is deciding, and when

A monster has two lives.

**Outside a fight** it stands where it was put, or keeps the orders its
script gave it: guarding a post or patrolling; a hostile one that has seen
the party lately makes for it. (The original also has it look
about and follow noises; the code does neither. See
*Open questions*.) It also follows a hypnotic
lure ([casting](casting.md) section 8). Outside a fight it is also *judged*, as
a group, every moment of play: whether it has noticed the party, what side its
group stands on, whether the group comes into a fight (section 2 and 3).
Wandering encounters come and go out of its sight (section 2).

**In a fight** it decides once at its moment, from what it sees then (round:
"anyone else decides now, from what they see now"). The decision is the first
of these that applies (retail `UpdateMonsterAI`, `BuildMonsterActionQueue`):

1. Cannot act: asleep or worse. It does nothing.
2. A bystander (the species the original keeps out of every quarrel, record
   0x224): it waits.
3. **Conduct** (section 4): run, withdraw, hold back, or fight on.
4. If it fights on: its **special attack**, if it has one ready; else a
   **spell**, if it casts; else **withdraw** if it cannot strike anyone; else
   **guard** a fellow, or **hold back**; else **strike**, which is the attack on
   [attack](attack.md) (and that page's *closing in*).

What its **ailments** may then do to the decision is section 8.

## 2. Out of a fight

### What a monster sees: the party

A monster's **eyes** are shut if it cannot take a combat action (asleep or
worse), or if it is Blind and does not fight blind (a Monk monster does).
Otherwise it makes out the party if all of the following hold (retail
`CanMonsterSeeMonster`, `UpdateMonsterSight`, `ComputeSightThreshold`):

- the gap from the monster to the party is within the level's **far clip** (the
  distance of the level's view);
- there is a clear line between its eye and the party's eye;
- the **sight threshold** reaches the gap. With `angle` the angle between the
  way the monster faces and the direction to the party:

  ```
  percent = 100                                  if angle < 45 degrees, or the gap is 0
  percent = (Senses - 2 * hider's level) * ((far - gap) / gap), cut to a whole number, otherwise
  percent = percent - 10 * concealment
  percent = (bonus + 100) * percent / 100        if its kind adds a sight bonus
  percent = clamp(percent, 2, 100)
  threshold = percent * far / 100
  ```

  It sees if `threshold >= gap`. *Senses* is the monster's own attribute (its figures,
  spells counted); *bonus* is the percent its kind adds to its sight. The
  *hider's level* is the lowest level among the party's members who can take a
  combat action (the party hides by its weakest able member); *concealment* is
  what Chameleon adds to the party's (its power). A monster is itself hidden from the party by its own level, with no
  concealment.

So a monster facing the party within 45 degrees sees it at any distance up to the
far clip; a monster looking away sees it only nearby, nearer the weaker its
Senses and the stronger the party's weakest member.

### What the party sees: a monster

The party sees a monster if the monster is alive and in the party's line of
view (within the far clip, a clear line between the party's eye and the
monster's eye, whichever way the party faces), and at least one member who can
take a combat action and is not Blind (a Monk sees blind) clears the same
threshold (retail `ComputeSightThreshold` for the party):

```
Senses = that member's Senses
hider's level = the monster's level (twice it is taken off, as above)
bonus   = that member's Scouting skill points, as a percent
concealment = the species' camouflage, outside a fight only (0 in a fight)
```

with the angle taken from the way the party faces, but 0 if Shadow Hound is
laid on the party. If the party is searching, the threshold is multiplied by
0.6667. This is what lets a fight open with the party surprised, or not
(round and [actors](actors.md)).

### What turns a group hostile

A monster group stands toward the party as **hostile**, **neutral** or
**friendly** ([actors](actors.md) *Side*). It starts as:

| The species is | The group starts |
| --- | --- |
| an NPC | neutral |
| of no faction, hostility radius 0 (hostile) | hostile |
| of no faction, radius -1 (neutral) | neutral |
| of no faction, a radius above 0 | neutral until the party comes within it |
| of the party's own faction | friendly |
| of another faction | by the party's standing with it: hostile, neutral or friendly band |

(retail `MonsterGroupCalcDefaultDisposition`). A group is then made to stand
some way, hostile, by:

- **Provocation.** A party member who acts against a monster (a blow aimed at
  it, a hostile spell cast at it, whatever the roll) turns its group hostile
  as the action starts. A dead monster, or one under Turncoat, provokes nobody;
  a group already hostile is not changed. The group is hostile for its
  **grudge**, below.
- **The party coming within the radius.** A group of no faction with a hostility
  radius above 0 turns hostile when its nearest member is within the radius of
  the party and its members make out the party. When such a group comes into a
  fight while neutral, it turns hostile as it comes in.
- **A fighting faction's leader seen.** While a fight runs, a group of a
  faction (not unaligned, not an NPC), not already hostile, turns hostile if
  its leader sees the leader of a *fighting hostile* group of the same faction.
  A group's leader is its first living member.

A stance set on a group lasts its **grudge**: the leader's Intelligence
squared, in game minutes (retail `RefreshMonsterGroupHostility`, `Intelligence^2
* 60` seconds). It also lapses at once if the party's standing with the group's
faction has moved since it was set. A group in a fight does not lapse. When it
lapses the group goes back to the stance its species gives it. A change in the
party's standing with a faction moves every group of it that nothing has set a
stance on.

How two bodies stand toward each other is on [attack](attack.md) section 2
(bystanders, neutrals, the same faction). A monster under Turncoat has its
stance turned about and has left its faction ([actors](actors.md) *Side*).

### Wandering encounters

A level has **generators**, spots in the level that call up a group of monsters
from a table now and then, out of the party's sight (retail `MonGen`). The
level keeps a **budget** of encounters that grows with time away and falls
when the party wipes one out, and a **limit** of how many may be about at once.

- **Timers.** A generator tries when its timer runs out. Its interval is its
  own, or the level's default, in seconds; each time it is set (and when the
  level is first made, partway through) it is `interval - jitter +
  below(2 * jitter + 1)` seconds, with `jitter` a fifth of the interval, cut
  down, and runs on play time. A generator
  that is switched off, or a level held by a fight or a conversation, does
  nothing. (A "harassing" generator never generates: the facts it waits on are
  not kept.)
- **Budget and limit.** On entering a level: the first time, the budget is the
  level's largest; coming back after `away` seconds, it grows by `away /
  period` (when the level has a period). The budget is held between the
  level's smallest and largest, then the limit is the budget held between the
  level's fewest and most. Past 36,000 seconds (ten hours) away, or at the
  first visit, every encounter about is retired and every generator, in a
  shuffled order, gets a forced try; otherwise the encounters about age by the
  time away. An encounter all dead is written off, and in a fight it costs the
  budget 1.
- **May this generator generate now?** In this order: nothing holds generation
  off and it is on; unless forced, its spot is 70 to 400 m from the party's eye
  and *not* in the party's sight (a line from the eye to a point a step above
  the spot is blocked); fewer encounters are about than the limit - if the limit
  is reached, encounters older than `lingers * 12` seconds and more than 100 m
  from the party are retired first, then the limit is asked again; unless forced,
  no monster stands within 10 m (each axis) of the spot; and its chance comes
  up: `below(100) < chance`, the generator's own or the level's default.
- **Which entry.** Roll below(100): 0-3 draws rarity class 3, 4-10 class 7,
  11-30 class 20, else class 70. The party's level is the party's average level
  plus 0.49, cut down. An entry suits if `off = |1 - challenge / party level|` is
  under 0.5, its rarity times `(1 - off * 1.8)` falls in the drawn class (at most
  3 is class 3, at most 10 class 7, at most 30 class 20, else 70), and its time
  of day fits (always, night, or day; night is before 5:00 or after 22:00). If
  none suits, any entry of exactly the drawn class and a fitting time does. One
  of those that suit is picked at random.
- **Companions.** Each companion the species lists comes along at its chance
  (`below(100) < chance`), as a group of its own with its own size roll, and
  retires with the encounter.
- **How many.** The species' group dice, leaned on (retail
  `RollEncounterGroupSize`): if the species' display level over the party's
  level is at least 1.2, then on Novice the fewest, on Expert as rolled, on
  Normal a roll repeated while above the middle of the dice. Otherwise, if the
  middle of the dice is above 0.8, as rolled; otherwise on Expert the most, on
  Novice as rolled, on Normal a roll repeated while under the middle. (The
  original compares the middle, not the strength, in this branch.)
- **Where.** At the spot, the members spread over the ground about it
  (a spacing of two body radii), facing the party; a member the ground will not
  hold shares the last spot found.

An encounter is *retired* when it is old and far (above) and the limit asks for
room. The party has to be rid of it in sight to be rid of it for good.

## 3. Coming into a fight and leaving it

Who is in a fight, the fight's order and its rounds are on [round](round.md);
what side a body is on is on [actors](actors.md). What is missing there:

### Coming in

While no fight runs, each group outside it, in the order the level lists them
(then any monster with no group), **comes in** when (retail
`MonsterGroupCanEngage`, `ShouldMonsterGroupEnterCombat`,
`AlertSameFactionGroups`):

- it is hostile **and the party has just acted against it** (provocation): at
  once, with all its living members; or
- it is hostile and a member of it **makes out the party** and a member **can
  act on it**. A member can act on the party when it is alive, can take a combat
  action, and one of its attacks reaches the party now, or its longest attack
  is touch or short and it can walk, or one of its spells aimed at enemies is
  castable with a target, or its special attack is ready with an aim
  (section 4); or
- a group that turns hostile by coming within its hostility radius is judged as
  a hostile group from then on.

While a fight runs, a group joins it when:

- it is hostile, a member makes out the party and **its leader is in reach of
  the party**: the leader's longest attack's reach plus one combat move, but
  never under 65 m (the engagement range 50 m plus a floor of 15 m); or it is
  provoked; or
- it is neutral, and it **stands with the fight**: one of its members (not
  camouflaged, in the party's view) is within 50 m of the party and stands
  roughly between the party and one of the fight's hostile fighters in the
  party's view - that is, `its gap to that fighter + its gap to the party < 1.15
  * that fighter's gap to the party`, and it is nearer the party than that
  fighter or nearer that fighter than the fighter is to the party (retail
  `MonsterGroupHasReinforcement`).

NPC groups never come in; they keep to their roles. Those who come in join the
fight, or open one: the party may be surprised, a camp is broken up, and the
party looks for whoever came ([round](round.md), retail `StartCombat`).

The Rust takes the original's path-length check as the straight distance, and
does not model its "a rendered member within extreme reach" clause (retail
`ShouldMonsterGroupEnterCombat`); see *Open questions*.

### Leaving

At each round's end, in this order (round: "groups with nothing left to fight
for leave"; retail `CheckMonsterGroupsLeaveCombat`), for each group in the
fight:

- a **neutral** group leaves unless it still stands with the fight (above);
- a **friendly** group (on the party's side) stays while one of its members
  sees an enemy of its own;
- a **hostile** group stays while a living member makes out the party or sees
  another enemy, and counts as engaged only while a living member makes out the
  party (the original also makes a group that has just come into the fight
  leave when its leader is out of reach and it cannot engage, see *Open
  questions*).

When no hostile group is left engaged, everyone leaves at once. A hostile
fighting monster outside any group keeps the others engaged, and itself stays
while any hostile group fights on.

## 4. At its moment in a fight: conduct and the order of its choices

### Conduct

Before anything else, a monster that has a mind of its own chooses its
**conduct**: **Flee**, **Withdraw**, **Hold**, or **Fight** (retail
`MonsterAdvanceChance`, `UpdateMonsterAI`). In order:

1. A **bystander** only ever holds.
2. The **chance to run** (percent), the first that applies:
   - an NPC that is not hostile and is at 33 percent of its health or less: 100;
   - it is already running: 100 while the party is farther than short reach
     (4 m), and then it goes on running; if the party is within short reach it
     stops running (0) and is cornered;
   - its species' own chance to run; if that is 0 and the species is **craven**
     (its Combat Morale is the first the editor offers), 100.
3. If `below(100)` is under the chance, it **flees**, and is running from now on.
4. Else, if its stance toward the party is **neutral**: it holds if its kind
   keeps its place hidden (**camouflaged**), otherwise it **withdraws**.
5. Else it **fights** (everything below).

To **hold** is to do nothing this round and have its breath given back
([round](round.md)). To **withdraw** or **flee** is to move straight away from
the party over the ground as far as one move takes it (section 6 gives the
move's length; a withdrawing or fleeing monster does not charge), announced as withdrawing or
fleeing - except that fear sends it running unannounced (section 8). A monster
that cannot walk, or that has moved this round, waits instead. A fleeing
monster that is fleeing goes on fleeing in later rounds while the party stays
beyond short reach.

### The order of what a monster that fights considers

1. **Its special attack** (a breath, or a summoning).
2. **A spell.**
3. **Withdraw**, if it can strike nobody.
4. **Guard a fellow**, or **hold back warily**.
5. **Strike** (the attack on [attack](attack.md) section 2, which also closes
   in or stands).

A monster that **prefers blows** (its species does not prefer range), has an
attack, and none of its attacks reaches the party, does not consider the special
attack or a spell: it closes in at once (the original sends it to approach, or
charge if hurt, before either; the Rust leaves it to the strike).

*Its attacks reach the party* means the gap to the party is within the reach of
one of its arms.

### The special attack

A monster with a special attack makes it at its moment if, in this order
(retail `CanMonsterFlee`, `UpdateMonsterAI`):

1. it has one, with a chance above 0, and it does not close in (above);
2. it is not cooling down: after a special attack the species' own number of
   rounds must pass, counting only the rounds in which it did not make one;
3. its stamina is at least a tenth of its maximum;
4. a Silenced monster is stopped only by a special attack that honours silence;
5. a **summoning** needs the party within long reach (25 m) and the summoner not
   friendly to the party, and is never what opens a fight; a breath needs an aim,
   as for a cone spell (section 5);
6. its chance comes up: `below(100) < chance`, where the chance is the
   species' if one of its attacks reaches the party, and **100 if none does**.

If so, it makes it: a summoning calls up its group (section 7); a breath is
aimed at one of its aims at random, costs it a tenth of its maximum stamina, and
catches whoever stands in its cone, whichever side (see *Open questions* 3). A
summoning costs the same. Its cooldown starts.

### The spell

A monster that casts and does not close in considers a spell (section 5). Its
chance to cast is the species' if one of its attacks reaches the party, and 100
if none does. It rolls this chance once, if it has any spell it can cast; if the
roll comes up it casts, otherwise the decision goes on.

### Withdrawing when it can strike nobody

A monster with no attack, or one that is Blind (and does not fight blind) and
has nothing in reach, withdraws, as above.

### Guarding a fellow

A monster whose species is a **bodyguard**, that is not Blind or worse, and has
an attack, and that has someone in reach of the party, may guard (retail
`BuildMonsterActionQueue`, `CanMonsterProtectCombatant`): with chance equal to
its Intelligence percent, it looks for a **ward**: a living fellow on its side
(friendly toward it), within the reach of its first attack, of a *higher level*
than it, and at **less than 40 percent** of its health. If there is one it picks
one at random and spends its moment guarding for the round: it takes blows
meant for the ward ([attack](attack.md) *Guards intercepting*). The original
also counts a fellow "out of formation" (see *Open questions* 15). A party member
under Turncoat is a friend of the monsters, and is a ward by the same rule.

### Holding back warily

A monster that is **wary** (its Combat Morale is the third the editor offers and
its Combat Behaviour is not the fourth) and has someone in reach of the party
adds *waiting* as one more equal choice to the attacks it considers, with
chance equal to its Intelligence percent ([attack](attack.md) section 2: "A
wary monster that chose to hold back"). If the pick falls on waiting, it is done
for the round and gets its breath back.

## 5. Choosing a spell

A monster has ten **spell slots**. Slots weigh in the choice 5, 5, 5, 10, 10,
10, 10, 15, 15, 15 (retail `g_spell_cast_weights`). It chooses (retail
`IsSpellUsableByMonster`, `CollectMonsterSpellTargets`, `MonsterSpellTargetOK`,
`ChooseMonsterSpell`):

1. **Which it may cast.** A slot's spell is castable if: the spell's record
   lets monsters cast it; the game resolves its effect; it is not aimed at the
   party, a point, an item or a lock; and it is not stopped by Silence (an
   Alchemy spell by a monster of a kind that keeps its Alchemy is not).
2. **Whom it sees.** It sees the living monsters in the fight with a clear line
   between its eye and theirs - itself among them - and the living members of
   the party **if its eyes make out the party** (section 2); a blind monster
   does not make out the party.
3. **At whom each castable spell may be aimed.** Each choice (an *aim*)
   takes in some bodies:

   | The spell is aimed at | The choices | What one takes in |
   | --- | --- | --- |
   | the caster | itself | itself |
   | a friend | each friend seen and in reach | that friend |
   | an enemy | each foe seen and in reach | that foe |
   | a group | each other group, the party or a monster group, aimed at the first of its members against the monster in reach (the party only if the monster makes it out; no sight is asked of a monster group) | its members against the monster |
   | a cone or a blast | each foe seen and in reach | every living body in its cone (within its reach and arc) or its blast (within its radius of the foe, with a clear line; the radius is taken at the power the monster's budget comfortably covers, where the original rolls a power each time it asks: see *Open questions*) |
   | all enemies | ahead, if any foe is seen and in reach | every foe |

   *In reach* is the gap within the spell's reach. **A choice is worth
   casting when at least half of what it takes in would feel the spell, and
   one does** (retail `MonsterSpellTargetOK`, `MonsterGroupHalfSpellTargetsValid`,
   `PartyHalfSpellTargetsValid`, one rule). A body would feel it when: a
   heal, it is hurt; a refresh, it is tired; a condition, it does not have
   it; an enchantment, it is not laid on it; a cure, it bears what the spell
   cures. A spell that does none of these (a damage spell) always would.
   A choice that would take in a living monster standing with neither side,
   other than the caster, is never cast (retail `SpellAreaHitsNeutralMonster`).
4. **The roll to cast.** Of the slots with a castable spell with at least one
   choice, the monster casts with its chance (section 4), rolled once.
5. **Which spell.** If it casts: below(total of the weights of the castable slots
   with a choice), walked down the slots in order. Then one of that spell's
   choices at random.
6. **Its power** ([casting](casting.md) section 3): from its budget.

A monster under Turncoat stands with the party
and against its former fellows, so the same rules choose its aims against
them.

## 6. Moving in a fight

A monster moves as its action: once in a round, at its moment; its next moment
is then twice the current one, no later than 100, and it acts again then
without deciding anew ([attack](attack.md) section 2). It moves when it **closes in** to strike, or **withdraws** or **flees**
(section 4). A monster that cannot walk (walking speed 0) does not move.

- **How far.** Its stride (its species' combat move range times 20 m), times 1.5
  if it charges (it is hurt: its health is 95 percent of its maximum or less);
  plus its stride times its Haste share (a tenth per power of Haste) when
  hasted, otherwise halved if slowed. (retail `SetUpMonsterTurn`)
- **Toward whom.** To close in: toward the party if it is hostile; toward the
  nearest foe it could strike if it is friendly to the party (turned) and can
  move; a neutral monster does nothing ([attack](attack.md) section 2). To
  withdraw or flee: away from the party's eye, over the ground, the whole move.
- **When it stops.** It stops when its move is spent - counted by walking time
  at its walking speed, not ground covered, so a blocked monster's move still
  ends - or when it arrives (within the reach of one of its attacks less 0.3 m
  of its target), or when it can no longer fight. The fight's next moment waits
  for it ([round](round.md)).
- **Charging.** A charging monster is announced as charging; one that closes in
  without being hurt as advancing; one that withdraws as withdrawing; one that
  flees as fleeing.
- A fight is **passive** when for two rounds nobody has attacked, cast, or been
  closing in ([round](round.md)); a hostile monster that still has to close in
  keeps it from being passive.

## 7. Summoning

A **summoning special attack** (a monster that calls its kind) is made when the
special attack is ready (section 4) with the party within long reach: the
summoner then spends a tenth of its stamina and starts its cooldown. The group
it calls is of the summoning's species, as many as the species' group dice
roll, but no more than nine (retail `SpawnSummonedMonsterGroup`).

- **Side.** Friendly if the summoner is friendly to the party; otherwise hostile.
- **Where.** If the party can see the summoner (within the far clip and a clear
  line to its eye), they come at the party's back (as the original's comment has it): in the
  direction of the party's heading plus `below(1000) * 0.0016 - 0.8` radians
  (about plus or minus 46 degrees), at three distances in turn - the first just
  outside the bodies (the party's radius, the member's radius, and 2 mm), then 3 m,
  then 6 m - taking the first for which any spot is found. If the party cannot
  see the summoner, they come about the summoner. They face the party. The
  spots are scattered over the ground, a body's width apart, clear of the party
  and of every living monster (the gap at least the radius plus half the
  spacing), with the ground not more than 5 m below the spot and nothing solid
  between. Those that will not fit stay away; if none fits, the summoning
  fails and the party is told so.
- **In the fight.** They join the fight at once and their group joins it too.

The summoning of an elemental by a spell is [casting](casting.md) section 8.

## 8. What ailments do to its choices

The conditions of [actors](actors.md) *Conditions* decide what an actor may do:
**asleep or worse** (webbed is not) takes its action altogether; **Blind**
shuts its eyes (section 2). When its action comes, and before it is done,
its ailments may take it (retail `GetConditionInterrupt`, `Combat.cpp:2940`;
a member's at `:2176`, a monster's at `:2429`), the first that applies:

| Ailment | What happens |
| --- | --- |
| Nauseated | 25 percent: it loses the action |
| Webbed | it breaks free with chance `Strength / 3` percent (the web is lifted; the action is lost anyway); else the web holds it and it loses the action |
| Blind | 20 percent, unless it fights blind (a member's trait, a Monk monster): it loses the action |
| Insane | 80 percent: if it is not berserk yet, 40 percent it **turns on its friends** if it can strike one (below); else it loses the action in one of three fits (below(3)) |
| Afraid | 25 percent, unless the action is a run (a member running with the party, a monster running for its life): it loses the action |
| Afraid, a monster | then 40 percent: it **runs for its life** and is fleeing from now on |
| Lured | a lure holds it: it walks toward the lure instead |
| Turncoat | a monster's action against its new foes stands, and is told. A member's action, whatever it was, becomes a strike at their fellow members: told (9) when it finds one; when it finds none they try their other weapon set (*items*, the weapon sets), saying so (string `0x23d`) when it lets them strike; failing that the action is lost (10) |

"It can strike one" is the strike's own search (retail's attack check,
`RateMonsterBestAttack`, `MonsterAttackReachesAnyone`,
`CanPartySlotAttackAnyTarget`, `Combat Range.cpp:77`, `:533`): a monster
looks among everyone in the fight, and needs to see them and a clear line
for the arm (`MonsterAttackReachesMonster`); a member turned or berserk
looks only among their fellow members (no monster scan). Insanity makes
that strike at friends: finding one it is berserk and strikes; finding
none it is a fit.

Each is told by its notice (retail `g_condition_interrupt_notices`,
strings `0x3a0` to `0x3ab`, by the numbers of `Interrupt`): Turncoat and
the lure too. Panic's number, 12, is past the end of retail's table, so it
is told by nothing.

An actor that **turns on its friends** is **berserk** for the rest of the
round (retail `pCombat->berserk`, cleared as the round is set up): whom its
blows seek is its friends, whatever it decided; and whenever an ailment
takes one of its later actions that round, it strikes at its friends
instead of losing it (`Combat.cpp:2461`), losing it only when none is in
reach. A member turned so strikes in the berserk mode when their best
hand's reach is short of long, else in their own (`:2207`).

An action an ailment took, or one made berserk, does not provoke whom it is
aimed at (`Combat.cpp:2238`, `:2479`).

An ailment that takes the action ends the actor's moment and the pause after
it. Fear, blindness and nausea do not change the *choice*; they take the
action it made. Blind eyes mean the monster sees nobody (section 5, 2). An
actor under **Turncoat** has the opposite side and so chooses foes among its
former fellows.

## 9. Open questions

Each place the code could not be explained as a rule was looked up in the
original (`MonsterAI.cpp`, `Combat.cpp`, `Combat Attack.cpp`, `MonsterGroup.cpp`,
`Sight.cpp`, `MonGen.cpp`, all under `src/wiz8/` of the decompilation). Those
that were the original's own rule are folded into the sections above and not
repeated here: the conduct and its numbers (`MonsterAdvanceChance`,
`MonsterAI.cpp:1034-1073`), the order special attack, spell, withdraw, guard,
strike (`UpdateMonsterAI`, `MonsterAI.cpp:817-945`), the slot weights
(`MonsterAI.cpp:141`), the special attack's conditions (`CanMonsterFlee`,
`MonsterAI.cpp:2279-2326`), the sight threshold (`Sight.cpp:197-243`), the
interrupts' numbers (`Combat.cpp:2940-3051`), the encounter draws
(`MonGen.cpp:217-285, 287-328, 457-517`), the grudge (`MonsterGroup.cpp:268-320`).
**Defect**: the Rust differs observably from the original. **Missing**: the
original has a rule the Rust lacks. **Unknown**: looked at, not settled. Rust
lines are in `src/game/`.

### Defects

1. **A hostile group leaves under a leader-reach clause the Rust lacks.** In the
   original a hostile group that has just come into a fight also leaves if the
   leader's reach plus one combat move (never under 65 m) is less than the
   group's nearest distance and the group cannot engage
   (`MonsterAI.cpp:2065-2152`, `MonsterGroupCanEngage` `:948-1031`). That clause
   runs only while the group's *engagement state* is set, a stored flag the
   original raises for the first moments of a group's fight and shares with its
   allied groups (`SetMonsterGroupEngagementState`, `MonsterGroup.cpp:1666-1695`,
   `MonsterAI.cpp:2570-2616`); the Rust has no such state. (The rest of the
   defect is fixed: a hostile group stays while a living member makes out the
   party or sees any other enemy, and counts as engaged only while a member makes
   out the party.)
2. **A ward may be out of formation.** In the original a monster guards a friendly
   combatant of a higher level and at under 40 percent of its health **or out of
   formation** (`CanMonsterProtectCombatant`, `Combat Attack.cpp:406-456`; it
   reads the *guard's own* modifier for a monster ward, `:448`). The Rust has no
   out-of-formation modifier on a body (the original sets it from spell effects,
   `Gameplay Mods.cpp:178-179, 419, 433`), so it considers health only. (The
   party-member half of the entry is not a defect: members are fight bodies like
   monsters, and one under Turncoat is a friend of the monsters, so it is a ward
   by the same rule.)
3. *Moved to Unknown 14.*

### Missing

5. **A monster outside a fight does almost nothing.** The original has a real-time
   decision for each monster outside a fight (`DoMonsterRTAI`,
   `MonsterAI.cpp:313-440`, `ChooseMonsterRTAIMode:443-650`,
   `ApplyMonsterRTAIDecision:650-776`):
   - A monster that faces the party (its orders say so) and can see it, within
     10 m, faces it.
   - A monster whose group has an incapacitated member does nothing.
   - Its **orders** are: *guard* (stand at its post, walk back to it if more than
     3 m from it), *patrol* an area around its place, *point patrol* (walk the
     points in turn, taking the next when within 3 m of the one it heads for,
     back to the first after the last), *random point patrol* (a random other
     point), and *face a direction*.
   - A monster that has just heard a noise (a startled group, Sonic Boom, Alarm)
     walks to it if the path to it is short enough (the noise's radius, times
     3/2, at most a cap, by hops), else turns to face it; and waits 30 ticks
     before it thinks again.
   - A monster that is resting (a pause) turns to a random heading about one
     time in twenty.
   - A monster that has seen the party within the last 240 seconds (sight
     "recent") and is hostile **charges** it, or, at 20 percent of its health or
     less, links to the party; a friendly or neutral one goes on with its orders.
   - A lured monster follows the lure.
   - A monster that cannot patrol is retired with its encounter, else waits 20
     ticks.
   The Rust has monsters keep their orders (guard, the three patrols, facing:
   [scripts.md](scripts.md)), walk toward a lure, and charge: every fit
   member of a hostile group above a fifth of its health makes for the party
   while one of the group sees it or saw it within 240 seconds
   (`world::pursuit`). The original moves the leader and the rest keep
   formation with it; it also stops short at nine tenths of its longest
   attack's reach, which is not there. **Noises** are there
   (`world::noise`): the party's steps are one every 4 m it walks, carrying
   50 m, or 75 m running or searching; a hostile group whose leader is not
   deaf and is within reach hears it on a roll under its senses plus half a
   point a metre of reach left, unless it has a louder one in mind; its
   Sonic Boom and Alarm are heard too, 50 m and 30 m more a point of
   power, where they are aimed (`assets/tables/noises.ron`). Its
   members walk to a noise if their leader has a way there over the navmesh
   shorter than the noise's reach, 50 m more for each step of the way that
   passes through a closed door, looked for no farther than one and a half
   times that reach and 250 m (`TestNoiseLineOfSight`, `NoiseHearingMargin`);
   otherwise they turn to it. They forget it once the leader is within 3 m. A round of fighting is a noise too, 100 m about the party, for
   every group not in the fight that is within two of its moves of it; the
   original brings a group that hears it
   straight into the fight if it could walk there in its move, where here
   it walks toward it and joins by the fight's own rule. Not there: after reaching a
   noise the original waits 30 of its thinking ticks, a rate it ties to its
   frame rate. One at a fifth of its health
   or less shadows the party instead of charging: it follows, keeping within
   twice the far clip of it (`MonsterLinkToStartupNavigator`). Resting turns and retiring an
   encounter that cannot patrol are there ([scripts.md](scripts.md)).
6. *Done:* sight's memory. The original keeps, for each monster, whether it
   sees the party now, saw it within 240 seconds or not (`Sight.cpp`,
   `MonsterAI.cpp:270-310`). Only the out-of-fight charge reads "within 240
   seconds" (`DoMonsterRTAI`), and a group here remembers when it last saw
   the party for that. Coming into a fight reads "now" alone
   (`MonsterGroupCanEngage` through `MonsterHasVisibleTarget`), as here.
7. *Done:* a hostile group joins a running fight only if its
   leader is within reach of the party both as the crow flies and by the way
   it would walk over its navmesh (`FindNavigatorPathDistance`), or if one of
   its members the party can see stands within extreme reach (50 m) of it
   (`MonsterGroupHasRenderableMember`, which despite its name asks whether
   the party sees the member, not the screen). A kind that keeps hidden is
   not counted; the original counts it once the party has detected it.
8. *Done:* a lured monster in a fight walks toward the lure at half its
   stride, haste and slowness aside, in place of its action (interrupt 11,
   after fear and before Turncoat).
9. **A monster's spell is re-judged when cast.** The original chooses at the
   start of the round and judges again, with a fresh power, when the action
   begins (`Combat.cpp:2587-2600`, `MonsterOKToCastSpell`). Here a monster
   chooses at the moment it acts, with the neutral test, so there is nothing
   to judge again; only the second roll of the power is not made.

### Unknown

10. **What counts as reaching.** The original rates a monster's attacks by
    whether any reaches *anyone* it may strike (`RateMonsterBestAttack`,
    `Combat Attack.cpp:257-278`), and sends it to close in on the distance to the
    target it would choose (`MonsterChooseTarget`); the Rust asks whether an arm
    reaches the party. They agree against the party; with a turncoat monster or
    a member under a spell that makes monsters friendly, they may not.
11. **The worth of a spell for a victim** is the Rust's generalisation of the
    original's list (`MonsterSpellTargetOK`, `MonsterAI.cpp:1366-1790`): see
    [casting](casting.md) *Open questions* 28.
12. **The opening surprise** of a fight (who is surprised when it opens; the Rust
    `opening_surprise`) was not read: it belongs to the round.
13. **Special attacks' own figures** (their effects, cones and the summoned
    species) are the table in `hits/specials.rs`, not read here.
14. **Who a breath catches.** For a monster's special attack the cone is collected
    by the cone rule of the spell targets, which for a hostile monster gives the
    party and the monsters on the party's side and for a turned one the monsters
    hostile to it (`PopulateSpellTargetMarkers`, `Magic.cpp:3750-3790`,
    `Targeting.cpp:3126-3160`). The Rust takes every body in the cone, its own
    fellows included (`combat/turn.rs` `breathe`). The function that carries the
    breath out in the original (`ExecuteMonsterSpecialAttack`) was not read, so
    whether the breath uses that rule is not settled.
15. **The blast an aim is judged by.** The original judges a blast's aim by
    who stands within its radius at a power it rolls for that one question
    (`ChooseMonsterSpellPowerLevel`, from `MonsterSpellTargetOK` and
    `SpellAreaHitsNeutralMonster`); the Rust takes the power the budget
    comfortably covers, before that roll's one up or down, so that looking at
    the party from outside a fight draws nothing.

## How this is built

The sections above are the rules. This says how the code is to be shaped
around them; where the code differs from it, the code is wrong.

**What a monster is ready for is a figure.** `SpecialReady` (a marker) says
its special attack may be made as far as the monster itself goes: one with a
chance, not cooling down, stamina at least a tenth of its most, not stopped
by a silence it honours. Its own system keeps it from `Special`,
`SpecialCooldown`, `Vitals` and `Conditions`; deliberating and coming into a
fight read it alike.

**A monster's spell slots are entities**, made with it as its arms are
(`SlotOf`): the spell, the slot's weight, its reach, the side it is cast at
and what one aim of it takes in are components written from its kind once.
That it may be cast now (section 5, 1) is a marker kept by its own system,
as `SpecialReady` is. The breath's cone is a slot of the monster too.

**A monster considering whom it could act on is an entity** (`Considers`),
with whether its eyes make out the party then (`SeesParty`), and a view of
each living body it considers (`ViewOf`): whether it sees the body (`Seen`)
and how the body stands toward it (`Toward`), each written once by its own
observer and read by its aims and its guarding alike.

**An aim is an entity**: one way a slot could be cast, at a body, a group,
the party or ahead, made when the monster weighs its slots for whatever the
slot reaches (it sees it and has it within reach; the party by the monster
making it out). What an aim would take in are entities of their own, one a
body (the body aimed at; a group's or the party's members against the
caster; whoever stands in a cone or a blast).

**One rule judges every aim** (retail `MonsterSpellTargetOK` and the two
`HalfSpellTargetsValid`, which are the same rule): an aim is worth casting
when at least half of those it takes in would feel it, and one does. A body
aimed at alone is half of itself. Whether a body would feel a spell is one
fact, written by one system from its `Vitals`, `Conditions` and what is laid
on it; an aim of a spell against enemies that would take in a neutral
monster is spoiled by another. Only sight tells members from monsters, as
the original does: a member is seen when the monster makes out the party, a
monster by a clear line, and a group spell reaches the party only when the
monster makes it out.

**A deliberation is such a consideration**, made at the monster's moment
(`Deliberates`) with the fight's combatants and the party's gap
(`PartyGap`), related to the monster it `Considers` and gone with it. It
goes by events in the order of section 4. Once `Begun`, its conduct and
whether it closes in are worked out side by side; then each step decides
the next:

| Step | What it reads | What comes next |
| --- | --- | --- |
| its conduct (`Conducts`), and `Fleeing` on the monster | `Standing`, `Vitals`, `Temper`, `Fleeing`, being an NPC; the gap | with whether it closes in (`ClosesIn`, from `Temper`, `Furthest` and the gap): a retreat, or `WeighsSpecial` |
| its special attack | `SpecialReady`, `Special`, `Standing`; for a breath, its slot's aims worth making; `Furthest`, `Vitals`, `Level` | the attack made, or `WeighsSpells` |
| its spells: the slots weighed, then the one cast | `CastChance`, `Slots` that are `Castable`; their aims worth casting and not spoiled; `Furthest`, `Level`, `SpellBudget` | the spell cast, or `CastsNothing` |
| withdrawing, guarding, holding back or striking | `Conditions`, `ActsThoughBlind`, `Furthest`, `GuardReach`, `Temper`, `Attributes`, `CanGuard`, `Level`; fellows' `Standing`, `Vitals`, `Level` | |

What it decided is said as `Decided`, and the deliberation is gone with
what it weighed. A slot is watched by the observer of its kind of aim (at
one body, a group, a cone or blast, or ahead), set as the slot is made, so
weighing a slot reaches only that one.

**What a monster makes out is kept on it.** Two markers, kept by one
system as monsters and the party move: its eyes make out the party
(`MakesOutParty`), and the party has it in view (`InPartyView`). A third,
`Reinforces`, says it stands between the party and a hostile fighting
(section 3, *stands with the fight*). Coming in, leaving, charging and
deliberating read them; nothing asks sight again.

**Every monster that fights is of a group**: a placed monster always is,
and only an NPC's body stands alone, which never fights. Coming in and
leaving are each one rule over groups.

**Coming into a fight is four rules**, each its own system, each marking
those who come in (`ComingIn`), and those marked are told together
(`Told`):

- *turning by a radius* (a hostility rule, beside alerting a faction);
- *provoked*: a group the party has just acted against;
- *opening a fight*: with none on, each fighter of a hostile group one of
  whose members makes out the party looks at the party as it would
  deliberate, weighing its slots at enemies and its breath, and the group
  comes in if one of them can act on it;
- *joining a running one*: a hostile group by its leader's reach or a
  member seen near, a neutral group by a member that reinforces, turning
  hostile as it comes in if its kind has a radius.

**Leaving** reads the same markers: a group stays while one of its
fighters reinforces (neutral), makes out the party or sees a foe of its own
(hostile), or sees a foe (friendly).

**A turn is facts of the actor, for the round.** Taking up its turn
(`TakesUp`) writes each as its own component: its next moment
(`NextMoment`), the attacks each hand has left and the hand due
(`AttacksLeft`), the weapons it accepted them with (`AcceptedWith`), a
readied Lightning Strike (`Lightning`, only on one who readied it). What the
round has done to it is a marker each, gone as the next round is taken up:
`Moved`, `MadeSpecial`, `Waited`, `Berserk`. A member's order is locked
(`Ordered`) by an observer of `TakesUp` set where the order is made (the
`Order` component's hook), so taking up a turn does not branch on kind.

**A moment is a chain of events**, each with one observer: `Moment`
revalidates (in the fight, able, its weapons the ones accepted, not
defending; otherwise its moment is `Done`); `Goes` fights on for one with
attacks spent or having moved; `Decides` is heard by an observer set where
the actor's kind is made: one with an order follows it (a breath order
checked by its own observer against `CanBreathe` and the cone), one with a
temper deliberates. Either way it ends in `Decided`.

**An action is an entity**, made by whoever decided (`ActionOf` the actor, a plain component: it lasts only the moment), who then triggers `Acts` at it.
What it is is which component it has: `Strikes`, `Retreats`, `Holds`,
`Protects`, `Breathes`, `Casts`; one aimed at foes is also `AtFoes`,
written by its maker (a cast's from its spell's definition). Each kind's
doer is set on the action as it is made (the kind's hook), and hears
`Unhindered`. Before that its ailments weigh it in the order of section 8,
each one event and one observer reading only its own sources:

| Step | Reads | Takes it as |
| --- | --- | --- |
| nausea | `Conditions` | lost, told |
| web | `Conditions`, `Attributes` | broken free (the web lifted) or held; lost, told |
| blindness | `Conditions`, `ActsThoughBlind` | lost, told |
| insanity | `Conditions`, `Berserk` | a strike at friends (`TurnsOnFriends`), or a fit: lost, told |
| fear | `Conditions`; the action a run | lost, told |
| panic | `Conditions`; a monster (`Temper`) | a quiet flight, `Fleeing` |
| lure | `LureInfluence` | a walk to the lure, told |
| Turncoat | `Conditions`, `AtFoes`; a member | a monster's stands, told; a member's becomes a strike |

A step either takes the action or passes it on; past the last it is
`Unhindered`. What took it is told by `Interrupted` (its notice's number);
a lost one is `Loses`, which ends the moment, but a monster `Berserk` this
round strikes at friends instead. A taken action does not provoke.

**Whether one can strike someone is the strike's own search.** An attack
being chosen carries whom it looks for (`LooksFor`) and, for a member
turned or berserk, that it looks only among their fellow members
(`AmongFellows`), set by whoever makes it; what follows `Found` or `Nobody`
is set on the attack by its maker. A monster's strike closes in when it
finds nobody; a member's tries their other weapon set
([items](items.md), the weapon sets). Insanity's strike at friends marks
the actor `Berserk` and is told when it finds one, and is a fit when it
finds none. A Turncoat member's strike is told as Turncoat when it finds
someone, and as unable to strike (10) when neither set finds anyone. One
striking without having taken up attacks this round (ordered to cast)
takes them up first. Each kind's strike is set where the kind is made, as
its deciding is.

**A moment ends by two facts**: `Done`, nothing left to it this round, and
`Shown`, it did something the player watches, holding the fight for the
pause. Each is one event with one observer. An interruption, a cast, a
retreat, a guard and a breath are both; a strike only `Shown`, its attacks
going on; holding back, a lost or refused turn, the party's move costing a
member its moment, waiting instead of striking and standing for want of a
goal only `Done`. Nothing else ends a turn or starts the pause.

**What a monster knows is read from its own components.** What its kind can do
was written onto it when it was made. No decision asks the species record.

## Effort

Source: `Health Stamina Mana.cpp`, `MonsterActionFatigueCost`, charged as
each action ends (`Combat.cpp`, `EndMonsterAttack`).

A monster tires by what it does in a fight, not by what it swings (it is
marked `Labours` as it is made). Each
thing done costs stamina once, twice over when it is diseased
(`assets/tables/efforts.ron`):

| What it did | Stamina |
| --- | --- |
| closed in at a charge; ran for its life | 5 + below(5) |
| walked up to strike; backed away | 3 + below(3) |
| held back; was drawn by a lure | 1 + below(2) |
| made an attack, whatever its swings | 2 + below(3) |
| made an attack in a fury (Berserk) | 5 + below(8) |
| cast, made its special attack, guarded a fellow, stood | nothing |

The original's names for its move actions do not line up one to one with
the moves here; charging and fleeing were taken as its dearer pair,
walking up and backing away as its cheaper.

## A group's leader

Source: `MonsterGroup.cpp`, `ElectGroupLeaderMember`; `MonsterManager.cpp`,
`RemoveMonster`.

A group has one leader: the first of it to be made. When the leader dies or
is gone from the group, the living member of highest level that is not
Turncoat or worse leads, the first of them among equals; if there is none
such, the first still living. The new leader takes up the script the old one
followed, from its top. The noise a group has heard is the group's, so
nothing of it changes hands.

The leader is the member that carries `Leads`, a one-to-one relationship to
its group. The rules that ask for a group's leader ask which member that is: who hears a noise and
walks to it, whose grudge keeps a stance, whose sight stirs its faction.

Not here: an encounter's companion groups electing a new leading group
(`ElectAlliedLeaderGroup`).

## Random encounter orders

An encounter table entry names the script followed by the main group’s leader.
An empty name means `Default.MSF`. Companion groups do not receive that script.
The orders belong to the leader and begin at its spawn position; unborn groups
retain the orders until birth. Source: `MonGen.cpp`, `MonGen::GenerateEncounter`,
selection of `script_names` and the main leader’s `SetScript` call.
