# Combat: entering, rounds, weapon attacks, ending

The rules a first combat slice needs: the party against one hostile group, weapon
attacks (melee and ranged) only, no spells and no party movement. Paths are
relative to `/home/agluszak/code/personal/decomp/whizzardry-8` (commit
`3d046808`). Abbreviations:

- `fight.cpp`, `turn.cpp`, `attack.cpp`, `actions.cpp`, `range.cpp`,
  `signals.cpp` = `src/wiz8/combat/{fight,turn,attack,actions,range,entity_signals}.cpp`
- `md.cpp` = `src/wiz8/monster_decisions.cpp`, `at.cpp` = `src/wiz8/action_targets.cpp`
- `ai.cpp` = `src/wiz8/local_code/MonsterAI.cpp`, `mm.cpp` = `src/wiz8/local_code/MonsterManager.cpp`
- `hsm.cpp` = `src/wiz8/local_code/Health Stamina Mana.cpp`, `im.cpp` = `src/wiz8/local_code/ItemManager.cpp`
- `ar.cpp` = `src/rules/action_rules.cpp`, `cs.cpp` = `src/rules/character_stats.cpp`
- `rec.h` = `include/wiz8/data/records.h`, `rr.h` = `include/wiz8/retail_records.h`

Character-side numbers (attack rating, # of attacks, swings, to-hit, to-damage,
initiative, AC) are in [characters](characters.md) §4; the monster record layout
is in [monsters-db](monsters-db.md). Strings were decoded from retail
`stringdata.dat`; record values were spot-checked in retail `monsters.dbs` and
`items.dbs`. Everything else is Whizzardry's reconstruction; the decomp was not
available. Open questions are marked **Uncertain** and collected in §9.

**Whizzardry's monster action names are misleading.** The strings they print
settle them: `ADVANCE` prints "%s flees for %s life" (0x23b), `APPROACH`
"advances" (0x236), `BACK_OFF` "charges!" (0x237), `RETURN_TO_START` "%s
retreats" (0x23a), `WAIT` "%s defends" (0x23c). This note uses those meanings:
**flee**, **advance**, **charge**, **retreat**, **defend**. The record byte
Whizzardry calls `advance_chance` (0x0e2) is therefore the **flee chance**, and
`flee_chance` (0x0e1) gates the special attack.

## 1. Entering and ending combat

### 1.1 What starts a fight

Outside combat, `UpdateMonsterGroups` visits groups in a staggered way (group
`i` when `i % 5 == tick % 5`; far groups less often). For an active group that is
not in combat and within 2 × far clip, it refreshes hostility, then calls
`MonsterGroupEnterCombat` if `MonsterGroupCanEngage` (`ai.cpp:94-161`). A group
can engage (`ai.cpp:661-738`) when all of these hold:

1. It is hostile.
2. A live member sees a hostile target in the party (`MonsterHasVisibleTarget`).
3. Some member can act. It needs a usable attack (§4.1: has the attack and a
   reachable target in range and sight), or an enemy spell it can cast, or a
   special attack. Failing those, its best attack is touch or short range, the
   party is within `combat_move_range × probe scale`, and a path from the
   leader to the party exists.

Entering gives every member a combat state. The next frame `HostilesEngage`
(any active hostile monster has a combat state, `game_advance.cpp:37-41`) calls
`beginCombat(0)` (`game_advance.cpp:94`). The player's combat toggle calls
`beginCombat(1)`, and then the party cannot be surprised (`mm.cpp:839`). Other
groups join at round boundaries through `ShouldMonsterGroupEnterCombat`
(`ai.cpp:1090-1180`).

### 1.2 Entry (`beginCombat`, `fight.cpp:114-229`)

1. Clear the per-fight state. Hold world advancement, so the game clock stops
   (§7).
2. `CheckMonsterGroupsEnterCombat`, then the "COMBAT!" notice (0x224).
3. `RollCombatSurprise(surprise)` (below).
4. Point the camera at the nearest hostile. Every combat monster gets no action,
   phase 0, and faces its nearest threat.
5. Party rows are reset; a character with 0 HP or condition ≥ Asleep is
   marked done for the round.
6. Save the formation; it is restored when the fight ends.

**Surprise** (`fight.cpp:1464-1503`). `surprise_possible` is the camping-ambush
mode, not ordinary play (`GameplayTime.cpp:297-337`). In ordinary play:

1. Unless the fight was player-started or a sight override is active, the
   **party is surprised** if no live, hostile combat monster is currently seen
   by the party (`party_threat.sight_state == SEEN`). In ambush mode the first
   able member must also pass `below(100) < SEN/2`, after a wasted `below(1)`
   draw (`fight.cpp:689-711`). Because `||` short-circuits, these draws only
   happen when a monster is seen.
2. If the party is not surprised: with the level's fast-movement (running)
   flag, `below(100) < 20` surprises it ("…due to running!", 0x23e). Otherwise,
   if the party was searching, `below(100) < 20` ("…due to searching!", 0x23f).
3. The **monsters are surprised** if none of them sees the party
   (`player_visibility.sight_state != SEEN`) and the mode is not ambush.
4. If both sides are surprised, neither is.

A surprised side gets phase 0 in the first round, so it does not act
(`fight.cpp:1170`, `fight.cpp:1201-1204`). A surprised party also does not wait
for orders: execution starts at once (`turn.cpp:192-204`). Both flags clear at
the end of the round (`fight.cpp:1293-1294`).

### 1.3 How it ends (`CheckCombatEnd`, `fight.cpp:1398-1458`)

It is checked between rounds and after each round (`turn.cpp:196`, `turn.cpp:385`).

- No participant left (participant = member, HP > 0, condition < Dead,
  `party.cpp:171-175`): "You've lost!" (0x230) and everyone still in the party
  is made Dead.
- Otherwise the fight ends when no active hostile combat monster is left (the
  after-round check passes `end_if_no_hostiles = 1`), or after 2 consecutive
  **passive, unengaged** rounds. A round is passive if every executed action was
  defend/protect (party) or defend/protect (monster); it counts as unengaged if,
  in addition, no group in combat `MonsterGroupCanEngage` (`fight.cpp:1295-1311`).
- With kills (`combat_result != 0`): "You've won!" and "%d kills".

`endCombat` (`fight.cpp:872-934`) then:

1. Removes Slowed from everyone, lifts Turncoat, and removes all enchantments.
2. Takes every group out of combat.
3. Awards experience.
4. Restores the formation and the auto-swapped weapons.
5. Opens a 2-minute "recent combat" window (`fight.cpp:903`).

**Experience.** Every hostile combat monster that dies, whoever killed it, adds
its `experience_override` (0x26b) to the fight's pool, or `experience` (0x181)
when the override is 0. A party killer also gets `kill_count += 1`
(`mm.cpp:325-363`). At the end, each participant receives
`pool / participantCount() + experience_bonus` (integer division). So
unconscious members get a share and dead ones do not (`fight.cpp:895-901`,
`party.cpp:259-270`). The notice is "Survivors earned %d Experience Points
each!!!" (0x231). `experience_bonus` only comes from prayer (+10).

**Loot is not dropped at the end of combat.** `DropMonsterLoot` runs when the
monster's die animation completes (`mm.cpp:949-960`), even mid-fight (§5.3).

## 2. Round structure

### 2.1 Choosing and executing

In turn-based mode the party edits its actions while `round_active` is set. The
player then starts execution (`BeginCombatExecution`). Continuous mode starts it
automatically (`turn.cpp:192-204`). Monsters decide **at execution start**,
after the party has committed. Each character's chosen action is
`committed`. When it starts, it is copied to `executing`, and it falls back to
the remembered basic action afterwards (`actions.cpp:195-205`, `turn.cpp:125-153`).

### 2.2 Execution start, in RNG order (`fight.cpp:448-590`)

1. `round_count += 1`. `round_counter` is the phase step (1 without party
   movement) (`party_movement.cpp:166-169`).
2. Each party slot 0..7 attacking with the Lightning Strike trait and a sword
   draws `below(100) < scaled 12` to get an extra-swing flag.
3. Each combat monster, in roster order, draws `reconsider = below(100) < 75`.
4. Each combat monster with HP decides its action (`md.cpp:750-876`, §4.1).
5. Phases are assigned (`AssignCombatPhases`, §2.3): party first, then monsters.

### 2.3 Initiative and phases (`fight.cpp:1162-1222`)

A round has phases 1..100. A low phase acts early.

```
party slot s (member, HP > 0, condition <= Webbed, party not surprised):
    v = below(4) + initiative                     # characters.md §4.4
    if committed action is Attack/Berserk:
        v += Hand[best hand].damage_bonus           # the weapon's initiative byte
    phase = 90 - clamp(v, -10, 89)                  # 1..100
    Snake Speed active and a hostile in combat: below(5) == 0 practises it
monster (alive, condition <= Webbed, side not surprised):
    phase = 90 - clamp(below(4) + record.initiative (0x0e4), -10, 89)
everyone else: phase 0 (does not act)
```

The best hand is the in-play hand that can reach, with the most `attacks`. A tie
goes to the larger `damage_bonus` (`attack.cpp:928-953`). The item byte at 0x048
that fills `damage_bonus` behaves as a **weapon initiative modifier**: retail
daggers are +2..+6, throwing weapons +4, axes −2..−14, and shields 0. An empty
hand uses `combined skill / 10` (`cs.cpp:227-243`). `phaseAtStep` changes
nothing without party movement (`party_movement.cpp:515-524`).

**Scheduler** (`turn.cpp:228-391`), one actor at a time:

```
round_counter = 1
loop:
    actor = first party slot 0..7 with phase == round_counter and able (HP>0, cond < Asleep)
         else first combat monster in roster order with phase == round_counter,
              alive, cond < Asleep, not motionless
    (a due but unable combatant gets phase 0)
    if actor: run its action to completion (§2.4), then loop again
    else round_counter += 1
    if round_counter > 100: end of round (§7), CheckCombatEnd, next round
```

**Ties**: party before monsters; then by party slot; then by roster order. Ties are
not rolled. A `combat_delay_ms` pause between actors is UI pacing only.

### 2.4 Several attacks per round

**Characters** (`actions.cpp:180-410`, `actions.cpp:412-463`, `attack.cpp:955-981`):

1. The first action records each hand's remaining attacks: `Hand[h].attacks`
   (# of Attacks), or 0 for a hand that cannot reach. The best hand starts.
2. Every **attack** (`StartCharacterAttack`) uses one of that hand's attacks
   and is resolved as 1..n **swings** (§3).
3. When the attack finishes, the next hand is the other hand if it has attacks
   left. It stays the current hand only if the other has fewer left *and* a
   lower initiative byte.
4. The next phase is `phase + (100 − round_counter) / (remaining + 1)`, where
   `remaining` is the next hand's attacks left. If it switched to a hand with
   more attacks left, the next phase is `round_counter` instead. A hand switch
   adds `init[current] − init[next]`. The result is clamped to
   `[round_counter, 100]`. With no attacks left (or > 100), the phase is 0 and
   the character is done.
5. So a character's attacks are spread evenly over the rest of the round.

If the first action fails (no target in reach), the character **defends**
instead. If a later one fails, it does nothing (`actions.cpp:370-384`).

**Monsters** (`fight.cpp:834-855`):

1. Committing an attack in the decision sets
   `attacks_left = record.attacks_per_round` (0x0e5) (`md.cpp:1150-1155`).
2. Each attack charges its fatigue (§5.6).
3. If attacks are left and `chooseMonsterAction(target_locked)` finds an
   attack, the next phase is `phase += (100 − round_counter) / (attacks_left + 1)`.
4. Otherwise, or at 101 or more, the phase is 0.
5. Target locking only holds when the monster's DEX < 75 and it is not
   berserk. Monsters with DEX ≥ 75 re-pick freely between attacks
   (`md.cpp:1055-1057`).

**Swings per attack**:

- Character: `below(Hand.swings) + 1 + item swings bonus (0x0ad)`
  (`attack.cpp:3277-3287`). Lightning Strike adds 3, or 4 if
  `below(11) + profession level ≥ 20`, to a hand-0 sword when not Slowed
  (`attack.cpp:3288-3302`).
- Monster: `below(swings_per_round (0x0e6)) + 1` (`attack.cpp:1850`).

A monster whose record has `attack_multiple_targets` (0x247) re-picks the
target between melee swings (`attack.cpp:1763-1780`).

### 2.5 Condition interrupts (`fight.cpp:1548-1650`)

Before every action of every actor, these are rolled in this order, and only
for conditions that are present. Each draw is `below(100)`:

- Nauseated: < 25.
- Webbed: < STR/3 breaks free; otherwise the action is lost.
- Blind: < 20.
- Insane: < 80, then sub-rolls.
- Afraid: < 25. A monster also rolls < 40 to flee.
- Lured.
- Turncoat.

With no conditions there are no draws. The slice can omit this but must keep
the hook.

## 3. A character's weapon attack on a monster

### 3.1 Target and mode

The target is the committed monster: picked by the player, or automatically by
`BestMonsterTarget` (`at.cpp:359-384`). Among hostile monsters the attack
reaches, it prefers the lowest condition, then monsters with a close-range
attack that can reach the party, then the nearest. No RNG is drawn. When the
target dies, every party aim at it is re-picked the same way immediately
(`mm.cpp:734-735`, `at.cpp:156-199`). The swings left in the current attack are
lost; the later attacks go to the new target (`attack.cpp:3856-3862`).

**Reach** (`at.cpp:255-286`, `ar.cpp:32-55`):

- The weapon's range category (item 0x047: 0 touch, 1 short, 2 long,
  3 extreme; empty hand = touch) is shortened by formation rows (§5.2).
- The monster must be seen, and its distance must be at most
  `CalcRangeDistance`: touch 1000, short 2000, long 12500, extreme 25000 world
  units (2/4/25/50 m) (`range.cpp:319-340`).

**Mode.** If no mode was committed, `ChooseAttackMode(item attack_flags)` runs
once per attack (`attack.cpp:3248-3252`). It draws `w = below(9)` and walks the
nine mode bits cyclically, taking the `(w+1)`-th set bit. This is not uniform
when the number of set bits does not divide 9 (`attack.cpp:174-191`).

### 3.2 Starting the attack (`attack.cpp:3220-3416`)

1. Mode (above). Swings (§2.4).
2. **Backstab**: the Backstab trait with a sword or dagger at touch range sets
   `natural_attack`.
3. **Unaware**: at short range or closer, if the monster is not facing the
   party, `unaware` is set when the party is "looking away from" it. A
   defending monster first gets a spot check: free the first time, then
   `below(100) < SEN − 25·attempts`.
4. Notice text. Melee: a voice line `below(3)`. Ranged (range ≥ long): fire the
   missile now (§3.5).

### 3.3 One melee swing (`attack.cpp:3418-3893`), in RNG order

The first call only plays the swing sound and returns. The next step resolves:

```
to_hit = GetTargetAttackAttributes(hand, mode, against target)        # §5.1
roll   = below(100) + 1
auto_hit = roll <= 5 ; auto_miss = roll > 95
fumble_chance = round((100 - to_hit_ignoring_defences)^3 * 1e-5)       # 0 if score >= 100
              * len(fumble_list) * 10 / 100, clamped 0..100
if !auto_hit and roll > 100 - fumble_chance:                           # fumble
    TargetHit = fumble_list[below(len)]  ; auto_hit = true ; roll = below(100) + 1
    unaware = natural = 0 ; damage later capped at max(1, HPmax/5)
else TargetHit = target ; auto_penetrate = auto_hit
if auto_miss: "Missed!" -> fatigue step
guardian interception (draws only if someone protects the target)
location = RollMonsterHitLocation(target record)                       # below(100)
if !auto_hit and roll > to_hit:
    miss; a near miss may be shown as "HITS SHIELD!" (no gameplay effect)
    unless blocked: voice draws below(100) (<30 if HP < 40%, else <25)
else:
    if paired ammo: mode = ChooseAttackMode(off-hand flags)            # below(9)
    pen = GetTargetHitAttackAttributes(hand, mode, location)            # §5.1
    if !auto_penetrate and roll > pen: "No penetration!"
    else damage = ResolveCharacterAttackDamage(...)                     # §3.4
         if damage == 0: "No damage"
         else apply to monster (§3.6); if it died with a top roll: below(2) voice
              critical/knockout/item effects (§3.4)
target fatigue: 1 miss, 2 hit without penetration, 3 penetrated        # no RNG
attacker fatigue: CharacterActionFatigueCost                           # §5.6, draws
consume ammunition/thrown item (ends the attack if it was the last)
```

The same `roll` decides both hit and penetration. A missed swing still rolls
the hit location. The fumble list is: the attacker, every other participant it
could reach, and every reachable combat monster except the current target
(`attack.cpp:2777-2816`).

### 3.4 Damage (`attack.cpp:2160-2306`)

```
dice  = the hand's weapon dice (a launcher uses its paired ammunition's dice;
        Punch = off-hand dice, Kick = primary dice; empty hand = Martial Arts table)
n = 1
if mode in {Swing, Thrust, Bash, Berserk, Punch, Kick}:
    if target is fleeing (monster) / running (party), out of formation, or unaware: n = 2
    if natural_attack (backstab): r = below(100); n += 1 if r+15 < c, 2 if r+75 < c,
                                  3 if r+95 < c    (c = Backstab chance scaled by level)
if mode == Berserk: n += 1
if n > 1 and target defends/protects (monster: defend or protect) and not unaware: n -= 1
n += 1 for each of weapon and paired ammunition whose slays_kind (item 0x061) == record.kind
rolled = sum of n rolls of dice                       # each roll = base + count × d(sides)
if n > 1: below(100) < 20n - 20 → voice line
damage = max(1, ((100 + bonus.damage_percent + Hand.damage_percent) × rolled + 50) / 100)
damage = ApplyDamageReduction:                        # attack.cpp:198-211
         r = monster.damage_reduction_adjustment + record.damage_reduction (0x0cf)
         if r != 0: damage = ((100 − r) × damage + 50) / 100 ; floor 0
```

`Hand.damage_percent` is the sheet's "To Damage (%)" (characters §4.7).
"for %dx damage!" (0x20d) reports `n`.

**Criticals and knockout** are attack effects rolled after damage lands, and
only when not fumbled (`attack.cpp:3707-3752`). The effect chances start from
the item's 16 effect bytes (0x050), plus the paired ammunition's; an empty hand
uses the Martial Arts hand's bytes. Then:

- **Kill** (instant kill, effect 5) gets `+ L/25`, and +1 more when
  `below(25) < L % 25`. `L` is the Critical Strike level for close combat; the
  Ranged Combat level for a Ranger (ranged-criticals trait) with a bow or
  sling; Critical Strike for a Ninja (thrown-criticals trait) with throwing
  weapons.
- **Knock-out** (effect 6) gets `+ scaled 5` for the knockout trait (Fighter).

`ApplyEffectConditions` then goes through the 16 effects
(`attack.cpp:2483-2760`). For each nonzero chance it draws `below(100) < chance`.
On a success it rolls the duration and then a saving throw
(`Magic Effects.cpp:429-475`):

```
resist% = clamp((target level − power) × 3 + base[condition] + resistance[realm], 5, 95)
          (scaled by difficulty)
resisted if below(100) < resist%
```

- `power = attacker level + min(level, 15)`.
- `base` for Dead is 24 and for Unconscious 22 (`Magic Effects.cpp:381`).
- Kill uses the Divine realm and Knock-out the Earth realm.
- A monster with `instant_death_immune` (0x1be), or whose kind is immune,
  resists outright.
- Effects stop once the target's condition is above Unconscious.

The slice can implement Kill and Knock-out and leave the rest at 0.

### 3.5 Ranged attacks (missiles)

For range ≥ long, **whether the shot hits is decided by missile geometry, not
by a roll**:

1. `FireCharacterItemMissile` aims at the target with `accuracy = to_hit` (the
   full §5.1 score) (`attack.cpp:3058-3110`).
2. `ScatterMissileAimPoint` (`attack.cpp:3112-3200`) draws `below(360)` for
   the direction and `roll = below(100) + 1`. With
   `d` = distance (world units), `eff = (100 − trunc(0.002·d)) × accuracy / 100`
   and `raw = 0.08·d`:
   - If `roll ≤ min(eff, 95)`, the aim point moves `roll/eff × raw`
     perpendicular to the line of fire.
   - Otherwise it moves further, up to about `0.2·d`. Blind attackers use a
     coarser profile.
3. The missile flies at about the range distance × 1.05 per second
   (`attack.cpp:3028-3035`).
4. If it collides with *any* living monster, that monster becomes the target
   ("retargeted"). A deflect chance (from effects) draws `below(100)+1 ≤ chance`
   only if it is > 0 (`signals.cpp:62-116`).

The swing then resolves (`attack.cpp:3517-3541`). A hit is `auto_hit` with a
fresh `roll = below(100) + 1`: there is no fumble and no 5/95 rule. It is not
auto-penetrating, except a Ninja's thrown non-sling weapon. Location,
penetration and damage then go as in §3.3–3.4. A miss is "Missed!". Each
further swing fires another missile.

**Slice**: without projectile physics, either intersect the scattered aim point
with the target's radius, or treat `roll ≤ min(eff, 95)` as a hit. The second
option is a deliberate approximation.

### 3.6 Monster damage and death (`hsm.cpp:214-298`, `mm.cpp:264-363`)

1. Guardian Angel absorbs first.
2. The attacker becomes the monster's `condition_target` (the killer), and
   `character_hate[slot] += amount`.
3. If `amount < hp`: `hp −= amount`, then fatigue `amount × 2/3`, then the
   struck reaction. The reaction plays the get-hit cycle and, if the monster is
   asleep, may wake it (`below(100) < SEN/2 + 50`) (`hsm.cpp:671-705`).
4. Otherwise `hp = 0` and `startMonsterDeath`, in this order:
   1. Start the die cycle.
   2. `deactivateMonster`: condition Dead; party aims at it re-picked; its turn
      ended.
   3. `recordMonsterKill`: XP pool, kills, faction offence.
   4. Remove it from its group.

   All of this happens before the next swing. A monster at stamina 0 becomes
   Unconscious, indefinitely (`hsm.cpp:588-613`).

## 4. A monster's attack on the party

### 4.1 Choosing the action, attack and target (`md.cpp:750-876`, `md.cpp:1009-1163`)

`updateMonsterDecision`, simplified for one hostile group:

1. Asleep or worse: no action.
2. `chance` = 100 for a non-hostile NPC at ≤ 33% HP, or for a monster already
   fleeing that is still within short range of a threat. Otherwise it is the
   record's flee chance (0x0e2), or 100 when that and the morale (0x1c0) are
   both 0. `below(100) < chance` → **flee**.
3. A melee-preferring monster (0x1b9 = 0) whose best touch/short attack cannot
   reach the nearest threat **advances**, or **charges** if HP ≤ 95%. The
   slice has no movement, so such a monster defends.
4. Then the special attack (gated by 0x0e1) and spells (0x0e0). These are
   outside the slice. When the monster has a usable attack the chance becomes
   100, so the record values need care.
5. Otherwise `chooseMonsterAction` builds a candidate list and picks
   `below(len)`:

```
if bodyguard (flag 0x04) and below(100) < INT: protect candidates (skip in slice)
if morale (0x1c0) == 2 and behaviour (0x1bf) != 3 and below(100) < INT: add "defend"
for each party index 0..7 in scan: avoided[i] = below(100) < Stealth(i) × 75 / 100
    # drawn for all 8 indices, even empty slots
for attack 0..2 usable (has, modes != 0, apr != 0, spr != 0, and reaches someone):
    for each participant i the attack reaches (range after formation, LOS, distance):
        if avoided[i]: resisted[i] = true
        else: one candidate per set mode bit of the attack
    (same for hostile-to-it monsters)
    every candidate added draws an unused below(100)                    # keep for order
for each resisted i: below(2) == 0 practises Stealth
pick = candidates[below(len)]
```

So the choice is **uniform over (attack, mode, character) triples** that reach.
An attack with more mode bits is chosen more often, and so is a character
reachable by more attacks. For melee, reach means the attack's range category,
shortened by formation rows from the monster's quadrant (§5.2), must cover the
monster's distance to the party (`range.cpp:195-210`).

### 4.2 Attack start and one swing

`StartMonsterAttack` (`attack.cpp:1825-1859`):

1. `attacks_left −= 1`. Re-check that the target still matches and is reached.
2. Swings: `below(spr) + 1`.
3. The announcement (`attack.cpp:1923-2056`):
   - `natural_attack` is set for record kind 3 at touch range.
   - If the target is a character not facing the monster, at short range or
     closer, `CharacterNoticesAttacker` runs. Only a defending character gets
     it: free the first time, then `below(100) < SEN − 25·tries`. If the
     character does not notice, `unaware` is set when the monster is behind it.
4. Start the attack cycle.

The swing resolves when the animation and missile have been waited out (§6).

`ContinueMonsterAttack` (`attack.cpp:1502-1815`) follows §3.3 with these
differences:

- `to_hit = GetMonsterAttackScore(...)`, and the fumble list comes from
  `BuildMonsterTargetList`. That list includes the monster itself, and, because
  of a retail comparison bug, the current target too (`attack.cpp:2818-2850`).
- The hit location against a character uses the **attacker's**
  `attack_body_part_chances[5]` (0x157). The order is Head, Body, Leg, Hand,
  Foot, and the fallback is Body.
- On a miss against a character:
  - The panic check runs only if HP < 50%: `below(100) < 15` queues a panic
    event (`fight.cpp:748-768`).
  - Then a voice line: `below(100) < 50`, then `below(3)`.
- Penetration uses `5·(attack_bonus + hit_bonus) + attack.score` + the
  penetration mode term − 5·AC at the location + the attribute term (§5.1).
- Damage (`attack.cpp:2389-2463`) uses the same dice count rules as §3.4 (the
  backstab chance comes from the monster level). Then:

  ```
  d = max(1, Σ n × RollDice(attack.damage_dice))
  d = ((100 − character.damage_reduction) × d + 50) / 100, floor 0
  ```

  The reduction practises Iron Skin. There is no Strength term for monsters.
- The damage goes to `Game::damageCharacter` (`live_party.cpp:523-600`). In
  rizzardry that is `Character::damage` (`src/game/characters.rs:512`):
  stamina −2/3·d, then HP, and death at 0. Retail also handles Guardian Angel,
  cheating death and waking a sleeper.
- If damage landed, the attack's 16 effect bytes (0x0ec) are rolled with
  `power = effective_level (0x24f) + min(level, 15)` and magnitude base
  0x102 + 0x22·i.
- A torso hit by a melee attack on a character wearing item 500 reflects the
  damage to the monster.

After the last swing, `finishMonsterAttack` (§2.4) charges its fatigue.

## 5. Shared formulas

### 5.1 Scores

SSDI = SEN + SPD + DEX + INT (effective). A monster's five attributes are the
record's 0x0d1 bytes in the order STR, INT, DEX, SPD, SEN, plus modifiers,
clamped to 1..125 (`Gameplay Mods.cpp:62-68`).

| Term | Character to-hit (`attack.cpp:2064-2124`) | Character penetration (`:2130-2154`) | Monster to-hit (`:2308-2383`) | Monster penetration (`:1672-1687`) |
| --- | --- | --- | --- | --- |
| Base | Attack Rating + 5·Hand.hit_bonus + 5·bonus.hit_bonus | + 5·(Hand.attack_bonus + bonus.attack_bonus) | attack.score + 5·mod.hit_bonus | attack.score + 5·(mod.attack_bonus + mod.hit_bonus) |
| Mode | table H | table P | table H | table P |
| Unaware / backstab | +10 each | - | +10 each | - |
| Fatigue | −penalty (½ for Modern Weapon) | −penalty (none for Modern) | −penalty (½ for "low-fatigue" verbs) | −penalty **only** for low-fatigue verbs |
| Re-pick | −max(0, 50 − s^0.75) | - | same, monster form | - |
| Defence | −5·AC(target, mode) | −5·AC_at(location) | −5·AC(target, mode) | −5·AC_at(location) |
| Attributes | +(own SSDI − target SSDI)/10 | same | same | same |
| Caps | Blind: at most 10 (Monk scaled) | - | Blind: at most 10 | - |
| Difficulty | ×1.4 Novice, ×0.6 Expert for the party; the reverse for hostile monsters (`(v·m·20)/100`, `DifficultyScaling.cpp:9-43`) | same | same | same |

- Fatigue penalty by band: 0/5/10/20/40 (`cs.cpp:508-524`).
- Re-pick `s` (character) = `SPD − 25·repicks − 50 + score`.
- The defence term uses the committed target for to-hit and `TargetHit` for
  penetration.
- The 18 low-fatigue verb ids are at `attack.cpp:429-432`.

Mode tables (`attack.cpp:639-690`). Table H applies to every to-hit score and
table P to every penetration score, whoever attacks:

| Mode | Swing | Thrust | Bash | Berserk | Throw | Punch | Kick | Lash | Shoot |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| H (to-hit) | 0 | −10 | 0 | −30 | −5 | +5 | −10 | +10 | 0 |
| P (penetrate) | 0 | +10 | −5 | −30 | +5 | −5 | 0 | −10 | +10 |

**Armour class** (`attack.cpp:983-1163`):

```
mod(monster) = mod.armor_class_adjustment + mod.armor_flat + fatigue_penalty/10
             − 2 if (unaware or backstab) and flag 0x02
             + 2 if it is defending
             − 4 if fleeing, not out of formation, and mode in {Swing,Thrust,Bash,Berserk,Punch,Kick}
mod(character) = (unaware or backstab: − positive Speed and Shield AC components)
               − 4 if praying (party not moving)
               − 4 if the party is running, as above
AC(monster)   = 10 − evasion(0x15d) + mod        (out of formation: mod − 5)
AC(character) = armor_class_total + mod
AC_at(monster, l)   = AC + mod + mod.armor_matchup − ac_by_mode[mode](0x16d) − ac_by_location[l](0x166) + evasion
                    = 10 + 2·mod + matchup − ac_by_mode − ac_by_location     # evasion cancels
AC_at(character, l) = AC + mod + armor_class_by_location[l] − armor_class_total
                    = 2·mod + armor_class_by_location[l]
```

The situational modifier is counted twice for penetration. Monster record
armour values are **negative for better armour**. For example, the Crocodile's
torso is −6 (AC_at 16) and the Rat's is +5 (5). A high evasion byte makes a
monster easier to hit: Rapax −4 gives AC 14, Slime +7 gives AC 3.

### 5.2 Formation reach (`ar.cpp:32-55`, `at.cpp:224-232`)

- Formation rows: 0 Front, 1 Right, 2 Rear, 3 Left, 4 Center, with 3 cells each.
- The monster's quadrant is its bearing from the party's facing:
  `((bearing + 45) % 360) / 90`.
- A row is occupied if someone in it is in formation.

For a character in row `r` and a monster in quadrant `q`, the number of
blocking rows is:

| Character row | Blocking rows |
| --- | --- |
| `r = q` | 0 |
| centre, or adjacent to `q` | 1 if row `q` is occupied, else 0 |
| opposite `q` | 2 if row `q` is occupied, else 1 if centre is occupied, else 0 |

Each blocking row shortens touch/short reach one step: short → touch → none.
Long and extreme are not affected. The same rule applies to both sides. With
the group in front, front-row melee reaches; the centre row needs a short
(polearm) weapon; the rear row cannot melee.

### 5.3 Treasure (`im.cpp:444-552`)

- **Non-NPC records** (flag 0x01 clear), for each of the 8 slots:
  1. If `count ≠ 0` and `below(100) ≤ chance` (so chance + 1 %),
     `k = RollDice(slot dice)`.
  2. Type 0 creates `k` copies of `item_id`.
  3. Type 1 calls `GenerateItemsFromTable(item_id, k)` once per k. **Uncertain**:
     up to k² items.
  4. Then the gold dice go straight to the party purse.
- **NPC records** drop their NPC's item list and gold.
- One item lies loose at the death position. Two or more go into a container
  (item 0x23c).
- Creating items draws RNG (`MakeItem`).

### 5.4 Hit locations

| Who | Table | Fallback |
| --- | --- | --- |
| Monster target | `hit_location_chances[7]` (0x15f), cumulative against `below(100)`. Labels for constitution 0: Wing, Tail, Head, Body, Leg, Hand, Foot. 18 records sum to 110: the excess is unreachable | 3 (Body) |
| Character target, monster attacker | attacker's `attack_body_part_chances[5]` (0x157) | 1 (Body) |
| Character target, character attacker | `gubLocalACPercent` = 15/40/30/10/5 (`cs.cpp:31`) | 1 |

Character locations are Head, Body, Leg, **Hand**, **Foot**: strings 0x429..0x431,
and `armor_class_by_location[3]` reads slot 5 (Hands). This settles the swapped
enum names flagged in characters §7.

### 5.5 Damage reduction

- Monster: §3.4.
- Character: `((100 − damage_reduction) × d + 50)/100` (`attack.cpp:302-316`),
  with damage reduction from characters §4.6.

### 5.6 Fatigue

- **Character**, per swing (`hsm.cpp:707-767`):
  - Punch: `below(3)+2`. Kick: `below(3)+3`. Empty hand: `below(4)+3`.
  - Weapon: `b = weight/40 + 1`, cost `below(b) + 1 + b`.
  - ×2 for Berserk, ×2 if Diseased.
  - It scales with load.
- **Monster**, per attack: `below(3)+2`, or `below(8)+5` for Berserk mode
  (`hsm.cpp:329-353`).
- The struck target loses 1/2/3 (§3.3).

## 6. Monster record fields used

| Off | Field | Used by |
| --- | --- | --- |
| 0x0cb | kind | slays_kind match; kind 3 = natural touch attacks; condition immunities |
| 0x0cf | damage_reduction % | §3.4 |
| 0x0d0 | flags: 0x01 NPC (loot), 0x02 (Whizzardry "vulnerable from behind": −2 AC when unaware, and can "block"), 0x04 bodyguard | §5.1, §5.3 |
| 0x0d1 | attributes STR INT DEX SPD SEN | SSDI, spot checks, protect/defend rolls |
| 0x0d6 / 0x0da | HP / stamina dice, rolled on the first visit (`mm.cpp:561-566`) | |
| 0x0e0 / 0x0e1 / 0x0e2 | spell %, special %, **flee %** | §4.1 |
| 0x0e4 | initiative | §2.3 |
| 0x0e5 / 0x0e6 | attacks per round / max swings | §2.4 |
| 0x0e7 + 0x22·i | attacks[3] (below) | |
| 0x157 | attack_body_part_chances[5] | §5.4 |
| 0x15d | evasion (i8) | §5.1 |
| 0x15e | constitution (location label row) | |
| 0x15f / 0x166 / 0x16d | hit-location %, AC by location (i8 × 7), AC by mode (i8 × 9) | §5.1, §5.4 |
| 0x176 | resistances[6] | saving throws |
| 0x181 / 0x26b | experience / override | §1.3 |
| 0x1b9 | prefer ranged | §4.1 |
| 0x1be | instant-death immune | §3.4 |
| 0x1bf / 0x1c0 | combat behaviour / morale | §4.1 |
| 0x1f3 | treasure: 8 × {u8 type, i16 count, u16 item, u8 chance, dice} (10 bytes) | §5.3 |
| 0x243 | gold dice | §5.3 |
| 0x247 | attack_multiple_targets | §2.4 |
| 0x24f | effective level (power, backstab chance) | §3.4 |

`W8MonsterAttack` (`rec.h:87-111`), 0x22 bytes:

| Rel | Type | Field | Notes |
| --- | --- | --- | --- |
| 0x00 | u8 | has attack | |
| 0x01 | u8 | weapon name / verb index | announcement; low-fatigue verb list |
| 0x02 | u8 | weapon type | 1 = use the mode's name |
| 0x03 | u8 | range category | 0..3. Retail: touch 617, short 135, long 107, extreme 80. A ranged animation on a touch/short attack is corrected to extreme, and the reverse to touch (`attack.cpp:1301-1309`) |
| 0x04 | u8 | attack score | 105..155 in the samples checked |
| 0x05 | u8 × 16 | effect chances | sleep, paralyze, poison, hex, disease, kill, KO, blind, frighten, swallow, possess, drain HP/stamina/SP, nauseate, insane |
| 0x15 | u16 | attack modes | bit = mode (§5.1 order) |
| 0x17 | dice | damage | i16 base, u8 count, u8 sides |
| 0x1b | u8 | effect magnitude base | e.g. poison strength |
| 0x1c | i8 | weapon class | impact sound |
| 0x1d | i8 | missile type | −1 none |
| 0x1e | 4 bytes | unknown | |

Checked: the Slime's (record 1) attack is score 130, mode Lash, `2+1d6`, 15%
poison and 50% nauseate. The Crocodile's attack 0 is Swing `2+3d9`.

## 7. Time

- **A round is 120 game seconds.** `AdvanceCombatRound` calls
  `AdvanceEnvironmentTime(120000)` (`fight.cpp:1312`).
- During combat the world clock is otherwise held. Game time reaches the clock
  only through rounds (`environment.cpp:87-92`, `fight.cpp:174`).
- The game clock ages timed effects once per 120 000 ms
  (`GameplayTime.cpp:156-163`), so **conditions and enchantments tick exactly
  one turn per round**.
- The 1.4 s passive stamina tick is suspended in combat
  (`GameplayTime.cpp:218-221`).

The other end-of-round work (`fight.cpp:1273-1395`), in order:

1. Clear both surprise flags. Update `unengaged_rounds`. Advance the clock.
2. Characters who defended, are asleep or are unconscious regain
   `2d(max(1, STMmax/20 − 1))`, ×3.3 scaled for the Fighter trait. Monsters
   that defended, are asleep or are unconscious regain the same without the
   trait.
3. Per character, award skill practice for the round (`fight.cpp:1226-1271`):
   per counted swing, `below(100) < 100` (50 for Martial Arts, 75
   dual-wielding), then `below(2 or 3)` picks the skill.
4. Count down special cooldowns.
5. Groups leave and join; sight is refreshed; invalid targets are re-picked.

## 8. Notes for the Rust model

- **Explicit pending state.** Model the round as `phase`, `round_counter` and
  a per-actor `next_phase` / `attacks_left` / `swings_left`. Keep the
  scheduler's tie order explicit: party slot, then monster roster order. The
  original waits on camera transitions, monster attack and die animations, hit
  sounds and missiles (`turn.cpp:60-90`). Do the gameplay resolution at the
  swing step, and let presentation gate *when* the step runs, not *what* it
  computes.
- **Death bookkeeping is immediate**: XP pool, re-aiming at a new target and
  removal from the turn order. Loot spawning is tied to the die animation in
  retail. A rewrite can spawn it at death, as an explicit deviation.
- **RNG.** rizzardry's `GameRng` is ChaCha8 (`rand_chacha`), not retail's MSVC LCG
  (`include/wiz8/rules/random.h:9-60`), so retail streams cannot be matched.
  Only the order matters, for rizzardry's own reproducibility. Retail draws
  **voice and notice chances from the gameplay stream**: the miss voice, the
  dice-count voice, the kill voice, the panic check and the unused per-candidate
  `below(100)`. AGENTS.md says presentation must not consume gameplay RNG, so
  moving these to presentation RNG is a deliberate, documented change.
- Retail `RollDice` can go negative with a negative base. rizzardry's `roll`
  floors at 0. Damage is floored at 1 anyway.

## 9. Uncertainties

- **Uncertain**: all of this is Whizzardry's reconstruction; the decomp was
  unavailable. No formula was compared with play.
- **Uncertain**: the missile hit test is physical. Missile speed, the party's
  collision sphere and the monster collision radii were not traced. §3.5 gives
  only the aim scatter.
- **Uncertain**: monster penetration subtracts fatigue only for low-fatigue
  verbs, while to-hit halves it for them (`attack.cpp:1676-1680` vs
  `:2321-2328`). This looks inverted; it is kept as coded.
- **Uncertain**: what flag 0x02 means. It both lowers AC when unaware and lets
  a monster "block" near misses. Blocking only changes the message and skips
  voice draws.
- **Uncertain**: `BuildCharacterTargetList` skips party member `i` when `i`'s
  *own* executing target is itself, not the attacker's target
  (`attack.cpp:2782-2790`). The character fumble path also passes the hand as
  the "action" argument.
- **Uncertain**: whether the party's distance to a monster includes radii
  (`GetDistanceToPlayer`). That changes touch reach at the margin.
- **Uncertain**: monster HP regeneration (0x17c, per game minute) during
  combat; it was not traced.
- **Uncertain**: the type-1 treasure loop passes the whole roll count to the
  table generator once per roll (`im.cpp:470-475`).
- **Uncertain**: surprise in camp-ambush mode. `beginCombat` resolves the
  ambush hold before rolling surprise (`fight.cpp:157-158`). Whether
  `surprise_possible` is still set when `PartyAvoidsSurprise` runs was not
  checked.
- Not covered: protect/guardian interception (`attack.cpp:470-612`); special
  attacks, spells and monster movement (advance, charge, flee, retreat);
  berserk and turncoat targeting; condition effects other than Kill and
  Knock-out.
