# Time, rest, recovery, and conditions

## Decision and evidence status

Use one `GameClock` resource for world time and its periodic accumulators. A
small `CampSession` owns the running rest sequence. Actors own their recovery
fractions, conditions, and enchantments in the profile that is currently
active. UI, sound, camera fades, and animation consume presentation time; they
do not independently advance poison, regeneration, or quest deadlines.

This is a proposed design, not implemented behavior. The comparison is pinned
to [wizardry-8-decomp 075cbd0][D] and [whizzardry-8 9c433ec][W]. Source bodies and
callers were inspected; no new retail or Rust runtime scenario was executed.
Recovered names such as `minutes`, `AgeMonsterSight`, and `UpdateCampFatigue`
need interpretation from their callers.

## What the two implementations establish

| Concern | Original and current evidence | Decision |
| --- | --- | --- |
| World advancement | Both accumulate world seconds with a millisecond remainder and deliver aging batches every 120,000 game milliseconds. Whizzardry separates interface holds from explicit pause. [D-clock][W-clock][W-holds] | Keep explicit clock domains and periodic remainders; derive pause policy from its actual reasons. |
| Combat time | Both advance environment time by 120,000 milliseconds at the round boundary. Whizzardry stops ordinary environment advancement while a party fight exists. [D-round][W-round][W-holds] | A round advances world time once, independently of animation duration or decision time. |
| Camping | Both have fade-in, accelerated waiting, and fade-out. The eight-hour deadline is conditional; incapacitation recovery has another exit. [D-camp][W-camp][W-clock] | Preserve a concrete sequence and explicit wake reasons. |
| Recovery | Both retain fractional recovery and distinguish camping, standing, movement, disease, traits, and combat. [D-recovery][W-recovery][W-stamina] | Recovery is a character rule, not a universal heal-per-second system. |
| Condition mutation | Whizzardry centralizes party infliction/lifting/aging and their derived-stat, death, targeting, and reaction consequences. Role conversions remain explicit. [W-conditions][D-conversion][W-conversion] | Expiration and cures call ordinary condition operations; profile conversion is not a second aging pass. |
| Absent monsters | Both perform whole-120-second catch-up on level entry, with a special recovery mode. [D-revisit][W-revisit][W-monster] | Age dormant world profiles once on activation; do not simulate absent actors each frame. |

## Clock domains and owners

| State | Owner | Meaning |
| --- | --- | --- |
| Real/presentation elapsed time | Bevy time and presentation playback | Input responsiveness, fades, audio, visual motion; each presentation system follows its own pause policy |
| World timestamp and calendar phase | `GameClock` | Game-world elapsed time; native representation can use integer milliseconds and an explicit calendar offset |
| Aging debt/remainder | `GameClock` | Whole 120,000-millisecond units plus retained remainder; an aging hold can defer application while debt accumulates |
| Hourly recharge and short recovery remainders | `GameClock` | Separate periodic rules; not independently advancing absolute clocks |
| Round/phase/current action | `Battle` | Combat scheduling; asks the clock to advance at the round boundary |
| Camp phase, start reason, deadline, fade identity | `CampSession` | Exists only while camping/recovery is running; its presence determines camp mode |
| Movement/rest history and environmental exposure | State on the party body | Shared recovery context derived from actual party movement; retains history that cannot be recovered from current velocity |
| HP/stamina/SP recovery fractions, finite conditions, enchantments | Actor's active `AdventurerState` or `WorldCombatState` | Rule state, preserved across supported saves |
| Global story/merchant deadlines | Their story/NPC owners | Compare against the appropriate world clock; the clock does not own the quest or stock |

Name the aging unit explicitly, for example `AgingTicks`. The source parameter
called `minutes` receives a count of **two-minute world-time units**, not elapsed
render minutes. The source also has a 1,400-millisecond stamina path outside
combat and a 3,600,000-millisecond recharge path specifically for item `0x266`
in party equipment, backpacks, and the shared pool. That is not evidence that
every charged item regenerates. [W-clock][W-stamina]

The 2,500-millisecond `UpdateCampFatigue` path is guarded by
`g_camera_sway_active`, not by camping. It can drain stamina and then HP, with
Android/equipment exceptions; combat also invokes it explicitly. Keep that
environmental rule separate until its mode/content mapping is established.
Do not turn a recovered helper comment into a claim that ordinary camping
causes this damage. [W-clock][W-fatigue][W-round]

### Pause is a policy, not several writable flags

Derive advancement policy from explicit player pause, the current interface
hold, active activity, `Battle`, and `CampSession`. World hold, party-input hold,
and an aging-only hold have different meanings. No second system owns its own
competing `time_passes` or `is_camping` truth.

Whizzardry's driver accounts for elapsed world time before applying a newly
requested interface hold. Its environment path advances the calendar and calls
the gameplay clock; combat instead supplies the round increment. Preserve that
ordering without the global pointers or cached booleans. A menu opening must
neither discard time already earned nor charge the time spent paused on resume.
[W-driver][W-environment]

UI and a camp fade must still be able to progress while party movement is held.
Likewise, waiting for attack animation does not add world aging on top of the
round increment. Conditions do not each run a Bevy timer against whichever
`Time` resource happens to be available.

## Walkthrough: camp, recover, and wake

1. **Validate the request.** Reject camping during party combat. Require the
   source-equivalent grounded/walkable state, no blocking movement, and an
   available interaction context. Opening a camp button is not the commit.
   The original world-cursor and busy-screen gates become concrete interaction
   predicates, not copied numeric screen commands. [D-camp][W-request]
2. **Begin the session.** Record ordinary requested rest or incapacitation
   recovery, clear explicit player pause, and retain any interface hold. Start
   the fade and camp presentation. The original `unengaged` test asks whether
   any party slot is eligible; it does not mean that no enemy is nearby.
   Automatic entry additionally checks that a character is active, the party
   is not fatigued, and movement/contact/UI permit it. [W-begin][W-eligible][W-clock]
3. **Wait for this fade-in.** When its specific completion is observed, enter
   the waiting phase. The source sets a deadline at current world time plus
   28,800 seconds, changes sight/navigation policy, and scales both game-time
   accumulation and encounter-generator duration from level data. This is
   the point at which accelerated rest is active. [D-camp][W-camp]
4. **Advance the world through normal rule entry points.** Recovery, condition
   aging, continuing effects, NPC timers, and encounter opportunities still
   operate. Camping is not an instantaneous assignment of maximum HP/SP followed
   by a calendar jump. Actual time passed and any inventory/story consequences
   are already committed. [W-aging][W-recovery]
5. **Choose an actual exit.** Ordinary rest reaches its deadline or accepts an
   allowed interruption. Incapacitation recovery ignores that deadline and wakes
   when a party member becomes eligible again; its acknowledgement command is
   refused outside combat. An ambush can end the hold through the combat path.
   Consequently, not every sleep lasts exactly eight hours. [W-clock][W-camp]
6. **Restore world policy, then complete wake presentation.** Reset the special
   sight/navigation/time-scale/generator settings, reverse the fade, and finish
   the session when that fade completes. For a surprised party entering combat,
   surviving members roll against Senses; failure applies the actual `Asleep`
   condition. Execute these gameplay consequences once, then update portraits,
   input, notices, and music. [W-reset][W-wake]

Cancellation does not rewind the elapsed clock, restore expired effects, undo
damage, or remove a potion already brewed. Pre-entry rejection creates no
session. A cancellation during a running session follows its allowed phase;
closing presentation must not silently finish or duplicate the wake operation.

## One aging delivery, with explicit internal order

`advance_timed_effects(ticks)` is an ordinary ordered operation. It first settles
movement/rest history, then ages eligible party profiles, processes relevant
battle effects, ages living active-map world actors, ages shared effects,
advances continuing spells, and processes NPC timers/pending consequences.
The original and current implementations have this broad ordering. Preserve
the swallowed-but-not-dead party exception when selecting eligible profiles.
Hypnotic Lure is reconsidered after continuing spell durations advance; see
[combat AI and navigation](combat-ai-and-navigation.md). [D-aging][W-aging]

For one adventurer, the inspected order is periodic damage, disease and other
special condition rules, signed recovery/drain modifiers, fractional passive
recovery, finite condition/enchantment aging, then the alchemist brewing rule.
Healing, damage, and expiration call their normal gameplay operations so death,
target availability, derived modifiers, and dependent state are settled before
the next action can use the actor. [D-recovery][W-recovery]

Some details matter enough to state directly:

- Base HP recovery uses scale 1 during the camp/surprise mode, 0.5 while resting
  outside combat, and otherwise 0 before trait adjustments. SP recovery has a
  different floor and trait/race modifiers. The short stamina path has its own
  standing/moving/fast-moving scales. One global rest multiplier is insufficient.
  [W-recovery][W-stamina]
- Fractions accumulate until an integer amount can be applied; full pools clear
  the corresponding fraction. Saving only current HP/SP changes later recovery.
  [W-recovery]
- Disease uses one deterioration trial against the delivered batch size.
  Calling a one-tick function repeatedly is not equivalent to the original
  multi-tick call. Continuing spell aging, by contrast, explicitly loops once
  per tick. Preserve each rule's batching contract and gameplay RNG order.
  [W-recovery][W-aging]
- The alchemist trait can produce an item while camping when its cooldown is
  zero; outside camping, aging decrements that cooldown. Invoke the inventory
  operation directly. A portrait or notice does not own the resulting potion.
  [W-recovery]

Represent condition duration as finite aging units or indefinite duration;
encode poison strength and source bindings as values. Expiration calls
`lift_condition`, including its modifier and targeting consequences, rather than
just removing an array entry. Indefinite conditions do not expire through
arithmetic. Some relationships, including swallowed/summon dependencies, also
require explicit cleanup when their source disappears. No condition entity or
generic timer-dispatch framework is needed. [W-conditions][W-bindings]

Recruitment/dismissal uses the explicit condition conversions described in
[party and RPCs](party-and-rpcs.md). The party-role and world-role pools have
different rules. Only the active profile ages; a retained adventurer sheet on
an off-party NPC is not another sleeping party member.

## Module boundaries and schedule

| Module | Owns/calls |
| --- | --- |
| `game::time` | Clock advancement, periodic delivery, derived advancement policy, camp sequence |
| `game::characters` | Recovery and condition rules for adventurer/world profiles; direct damage, cure, and expiration consequences |
| `game::combat` | Exactly one world-time increment at the completed-round boundary; surprise battle/wake coordination |
| `game::magic` | Continuing spell/field aging outside and inside combat; source dependency cleanup |
| `game::world` | Movement history, grounded rest eligibility, encounter timing, active-map presence and revisit catch-up |
| `game::inventory` / `game::story` | Item recharge/brewing ownership and NPC/story timer consequences |
| Presentation/UI | Camp command and feedback, fade identity/completion, visual/audio clocks |

At the schedule boundary, credit the elapsed interval under its previous policy,
then establish the newly requested policy for the next interval. Explicit round
increments use the same clock operation. Deliver due work in the stated order.
**Do not infer the interval's policy from the activity present when a later
system happens to observe the new clock.** In particular, completing the final
combat round must settle its 120-second increment as *combat* before removing
`Battle`; otherwise a later recovery system may interpret it as exploration.

The clock and aging operation must account for the interval once, including
any retained fractional remainder or deferred debt. The debt carries its
original advancement policy if it can cross a mode change. A system-local
`last_timestamp` used only to detect elapsed time must not be the sole
authority for delivered gameplay effects. A plain function called by the
owning transition is preferable to another event/observer pipeline.
Individual rules borrow only the profiles and data they need. A huge `Game`
argument, a generic event chain, or a universal timer service would obscure this
order.

## Save, unload, and revisit

Persist clock remainders/debt, relevant movement/rest history, actor recovery
fractions, finite/indefinite durations, dependency IDs, cooldowns, and gameplay
RNG state at the [supported save boundary](persistence-and-travel.md). Initial
native saves exclude an active camp sequence; manual requests report that gate,
and autosave waits. A future camp checkpoint would need a defined phase and
wake state, not a serialized animation handle.

On map departure, capture the timestamp used for dormant aging. On activation,
the source computes whole elapsed 120-second units and calls monster aging in
its special recovery mode. That mode selects a recovery scale of 1; it does
**not** mean “instantly restore full health.” Despite its name, `AgeMonsterSight`
also handles damage, HP/stamina recovery, conditions, enchantments, effects, and
movement bookkeeping. [D-revisit][W-revisit][W-monster]

Ordinary absent actors live in dormant records. Campaign-lived named NPCs retain
their authoritative world profile but have no active simulation presence. Apply
the same revisit aging to that profile once; do not both update it off-map and
catch it up on entry. Do not age its retained adventurer profile as well. Global
stock/quest deadlines retain their own rules. Encounter-budget catch-up and the
ten-hour encounter reset are separately described in
[persistence and travel](persistence-and-travel.md).

## Bounded questions and acceptance scenarios

Resolve these before declaring timing parity:

1. Characterize the frame-to-world scaling and aging-only hold release with
   recorded source inputs. A deterministic native simulation cadence is a
   deliberate choice; disease batching prevents assuming all elapsed-time
   subdivisions produce identical results.
2. Identify the content meaning of camera-sway environmental fatigue and its
   explicit combat calls before assigning a user-facing name or consolidating
   its counters.
3. Characterize sub-tick dormant elapsed time across repeated short revisits.
   The inspected entry function floors the elapsed duration; do not invent
   accumulated remainder behavior without checking the departure timestamp rule.

| Scenario | Required result |
| --- | --- |
| An update crosses an aging/recharge threshold with a remainder. | Deliver the correct count once and retain the remainder; aging holds retain owed work. |
| A menu opens just before a threshold and remains open. | Account for pre-hold time once; no paused-time recovery or resume burst. |
| A combat round takes different amounts of presentation time. | Each completed round advances world time by the same 120,000 milliseconds. |
| Normal camp ends, is cancelled, or is interrupted by enemies. | Keep elapsed recovery/damage; restore overrides and resolve wake once. |
| All eligible members are incapacitated but some remain active. | Use the recovery entry/exit rules; do not force the ordinary eight-hour exit. |
| A diseased character receives a multi-tick delivery. | Match the batch trial and RNG consumption, not an assumed repeated-loop equivalent. |
| Poison expires or a dependency source is removed. | Run its actual lift/cleanup operation and settle derived state before later actions. |
| Save/load occurs with fractional recovery and clock remainders. | The next supported update produces the same rules and RNG outcomes. |
| An off-party named NPC's map is revisited. | Age its world profile once, leave retained adventurer state inactive, and create no duplicate actor. |

## Pinned source references

[D]: https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819
[W]: https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873
[D-clock]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L97-L189
[W-clock]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L147-L241
[W-holds]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L81-L109
[D-round]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat.cpp#L1862-L1884
[W-round]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/fight.cpp#L1522-L1550
[D-camp]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L196-L316
[W-begin]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L117-L141
[W-request]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L247-L269
[W-eligible]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/fight.cpp#L257-L268
[W-camp]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L293-L370
[W-wake]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L375-L458
[W-reset]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Sight.cpp#L58-L64
[D-aging]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L497-L586
[W-aging]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L512-L606
[D-recovery]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L593-L901
[W-recovery]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L611-L920
[W-stamina]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L1342-L1420
[W-fatigue]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L1288-L1336
[W-conditions]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/live_party.cpp#L244-L330
[D-conversion]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/ConditionsAndEnchantments.cpp#L823-L867
[W-conversion]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/ConditionsAndEnchantments.cpp#L397-L439
[W-bindings]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/ConditionsAndEnchantments.cpp#L68-L109
[D-revisit]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Sight.cpp#L366-L385
[W-revisit]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Sight.cpp#L293-L316
[W-monster]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L1121-L1280
[W-driver]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/game_advance.cpp#L53-L111
[W-environment]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/world/environment.cpp#L90-L172
