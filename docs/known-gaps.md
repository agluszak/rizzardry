# Known gaps

What each implemented feature does, what it leaves out, and where it
departs from retail, in rough order of the
[implementation plan](implementation-plan.md)'s first slice. Keep an entry
next to its feature when that feature changes.

- Character review uses retail artwork, bitmap fonts, character portraits,
  statistics/skill layouts and page controls. Conditions and represented item
  effects can be filtered and scrolled. Spell review, level-up editing, trait and
  enchantment details, several combat/armor values, and the improved-skill palette
  remain unported. See [source comparison](research/character-review.md).
- Rendering and input no longer bootstrap gameplay: `LevelDefinition` and its
  collision/trigger/placement loader are separate from `LevelGeometry` and
  presentation loaders. Headless play installs no renderer, render assets,
  input bindings, UI or audio. Visuals still share the same compiled package;
  this is a runtime/dependency ownership boundary, not a Cargo feature split.
- The navmesh cache keeps its small hand-written codec, which a standard codec
  would not make simpler.
- Doors: a locked door with a key opens for that key in hand (unlocking
  it for good, kept with the level) and otherwise runs its fallback, as
  retail's `SelectAction` does; a door locked without a key stays shut,
  since retail's lock picking and traps are not ported. Two retail
  containers name loot tables that do not exist (the camp's chest and
  Rapax Upper Floor's `CONSAFEDOOR`); like retail, they generate nothing.

- Monsters take sky light while they can see the level's `SUN` light,
  fading as retail does, plus the level offset and nearby static lights; but
  those lights reach the whole actor by their falloff at its centre, not per
  vertex with N·L, and their runtime attenuation is not confirmed. Monsters
  use only the nearest detail level and ignore particles, sounds and skins.
- Elevators/lifts and random teleports are not implemented; camera paths
  fly the party but retail's level-specific reactions when some finish
  (Monastery 1's cameras, Arnika's) are not. Arriving does not probe for the ground (the party settles).
- Traps (`0x24`, each living member hurt by their own roll) and damage
  zones (`0x23`, 1d6+2 every half second while the party stays) hurt the
  party. Damage drains two thirds of itself from stamina; running out of
  stamina knocks a member unconscious until it climbs above 9, and 0 HP is
  death (derived from health, not stored as a condition), which clears
  other conditions but infatuation. The HUD dims the unconscious and shows
  the dead as their race's skull.
- The party recovers as game time passes (`game::party::recovery`):
  stamina every 1.4 game seconds at `(0.9·max + 20)/240`, health every
  120-second aging tick at `(0.4·max + 20)/240`, both halved while standing
  still; moving recovers a tenth of the stamina and no health, and camping
  recovers both at full rate. Combat rates wait for combat.
- Camping (`game::camp`, C as in retail's `mgskeyboard.ini`): only
  while still; a second's fade into retail's camping screen, eight game
  hours at 2880× (ten real seconds), a second's fade back; Escape cuts it
  short. When every living member is incapacitated the party is forced to
  camp until someone can act. Notices use retail's strings. Ambushes, the
  waking Senses roll and camping music wait for combat and music work.
  Exploration input is held, in one system, while the automap, a travel
  question or a camp exists.
- Combat, first slice (`game::combat`, rules in
  [combat](research/combat.md)): monster species from `monsters.dbs`;
  placed monsters roll health and their size within their model's scale
  range. Non-NPC monsters engage by retail's sight rule (far clip, line of
  sight eye to eye, `ComputeSightThreshold`) when an attack reaches or their
  best attack is close and they can walk; the view turns to the nearest. In
  the fight they get walking bodies (fight radius, scaled speed) and close
  in through the navmesh continuously, where retail schedules advance and
  charge actions within the round (not yet). Reach is retail's
  `GetDistanceToPlayer`: feet to the party point 2 m under the eye, less
  both alternate radii. The party may walk away, as in retail's continuous
  mode, and cannot camp. Rounds follow retail's initiative
  phases (party first on ties, then accepted roster order, then monster order).
  Members use the live formation to choose among reachable targets, and
  monsters pick uniformly among the (attack, mode, member) choices that
  reach. Swings use retail's to-hit and penetration roll, hit locations,
  armour and damage reduction; death bookkeeping happens at the blow
  (experience pool, gold, out of the turn order) and the dead play their
  death cycle. A round is 120 game seconds and the clock is otherwise held;
  the fight ends when no hostile is left or after two rounds without an
  attack, sharing experience among the living. Fights are phased by default as in
  retail: the party's orders (Attack or Defend; A and D) stand, and each
  round waits for Enter or the round button; backslash switches to
  continuous. Actor-owned orders and weapon choices are accepted at round
  start; edits affect the next round. Defending gives +2 armour and stamina
  back at the round's end, in accepted actor order. Sleeping and unconscious
  members also recover; Fighters receive their current-profession-level-scaled
  regeneration bonus. Not yet: party movement,
  additional orders, spells, fumbles, criticals, effects, the off hand, monster
  movement and fleeing, factions, levelling. A slain monster leaves its
  treasure (retail `DropMonsterLoot`: each slot's chance and count, table
  slots drawing that many items, then its gold): one item lies where it fell,
  several in a Bag of Items. Retail drops them with the dying animation,
  here at the blow; NPC monsters' own item lists are not ported; and retail
  opens a bag in its item picker, while here a click empties it into the
  party's pool, as chests do.
- The main game screen (`presentation::main_screen`, layout in
  [main game screen](research/main-game-screen.md)): retail's roof, party
  columns, bottom panels and button-row sockets around a 3D view that
  renders into the layout's viewport; the Closed, Wide and Full modes
  (roof buttons, Backspace) and the panel arrows with retail's rules;
  members' medium portraits (skulls for the dead, dragons for empty
  slots), hands, average armour, names and HP/stamina bars drained from
  the top; selection by click, previous/next and keys 1-6, a double click
  opening the inventory; messages in the text box (retail channels, wrapped,
  Combat while fighting, following new lines, scrolled by its arrows and
  PgUp/PgDn/Home/End, emptied by Del); the radar (compass rim turning with
  the party, monster and item blips placed and coloured as retail does) and
  the button bank (Camp, Search and Formation working), and the formation
  board; retail's arrow cursor. The bank's other buttons are still to come.
- The inventory screen (`presentation::inventory`, I; layout in
  [review items](research/review-items.md)) shows a member's equipment on
  their race's statue with retail item pictures and counts, the backpack
  (red where they cannot use an item), the pool and the gold; the header's
  portraits switch member, Esc or Exit closes it. Clicking picks items up
  and puts them down (swap, merge into a stack, deposit in the pool); the
  held item follows the pointer and is put away on closing. Shift-click
  splitting, ammunition pouring, stowing a weapon's partner, the info
  remaining item-action buttons and filtering are still to come.
  Plane-shaped triggers (falls) are not decoded as places yet.
- Monsters go by their look ("Crustacean") until the party identifies
  their group (`game::world::identify`, retail `DetectMonsterGroups`): on
  each aging tick and when a fight starts, every group with a member in
  view rolls each member's recognition (three per kind seen beyond its
  level, a tenth of Intelligence, Mythology), and the best margin names it
  ("Ragnar identifies the Crocodiles:", "They're Ironskin Crocodiles!").
  Each group seen adds its members to everyone's count of its kind once.
  Retail asks whether a member is drawn; here it is within the far clip
  and retail's 85° view. Mythology practice is not modelled.
- Items lie about each level as retail's level state (`.sts` `ITEM`) places
  them (`game::world::items`): one entity owns each, shown with its
  `data/items3d` model (its own or its kind's, else the question mark),
  turning a sixteenth of a turn per 50 ms and lit like monsters. Clicking
  one within 10 m with an empty hand takes it into the hand; the held item
  follows the pointer on the main screen too, and clicking a portrait gives
  it to that member, or the pool when they have no room (the pool first in
  a fight). Clicking in the view with an item in hand, off any prop, drops
  it: thrown along the pointer's ray at most 5 m, resting a quarter metre
  above the ground (retail aims by its own throw angles). Clicking in the
  world goes through the pointer, as in retail, and only inside the view.
  Hidden items come to light when searching finds them
  (`game::world::search`, retail `Search.cpp`): every half second the best
  searcher (half their Scouting plus a Senses bonus; rangers always, others
  in search mode, S or the magnifier) spots hidden items in front within
  0.2 m per point of score and in sight. Search mode pauses for a fight.
  Not yet: searchable triggers, skill practice, turning the view to the
  find, the identify attempt, retail's rare search-mode mishap. Hovering an item
  within reach pulses it green and, after half a second, shows its name in
  retail's help box ("Bullets (Unidentified) (16)"); unidentified items
  go by their kind's name everywhere. Containers' item picker is still to
  come.
- Stamina and healing fountains (`0x25`, `0x26`) restore every member by
  `data[2]`, with `data[0]` uses (−1 endless) refilled `data[1]` × 720 game
  seconds after use. Spell-point and spell-casting fountains (`0x27`–`0x2b`)
  wait for spells.
- Most trigger actions are not performed yet (lights, missiles, fountains,
  timed actions, required items). A level the party leaves
  is kept as a record (`LevelRecord`) that its next activation consumes:
  what its triggers keep (switches, positions, locks, contents, fountains,
  variables; not proximity, animation waits or zone timing), where its
  props were heading by definition index, its monsters as they were (the
  dead stay gone), its lying items, what the party knows of its groups,
  and its map notes. Only for the session: not saved with the game, and
  dormant-level catch-up does not exist yet.
- Map notes belong to their level (retail's `AUTO` chunk) and are not saved
  with the game; the automap does not yet track explored cells or
  project notes onto a minimap. Only the map screen exists.
- The screenshot tool intermittently omits all or some unchanged UI artwork,
  even while map/note assertions pass. Complete map frames have also been
  captured; the cause is still unresolved.

## Attack stamina follow-up

Ordinary party attacks spend stamina after every executed swing, including misses
and the killing blow. Punches cost 2–4, kicks 3–5, other empty-hand attacks 3–6;
weapons use their weight band. Disease doubles the cost. Exhaustion takes effect
through Vitals immediately and stops the remaining swings. Later swings use the
attacker's reduced stamina in their hit calculation. Shared-load scaling and the
separate Berserk order are still absent; a weapon's Berserk mode alone does not
select that order. Monster attack fatigue is also still pending.

Ownership: the accepted battle work identifies the actor and weapon. The attack
operation calculates each executed swing's cost and applies it to that actor's
Vitals before continuing. No pending-fatigue message, extra entity, or copied
character state is involved. The cost draw now occurs between swings, so seeded
combat outcomes change from the previous once-per-attack approximation.

Both sources agree: `wizardry-8-decomp` at
`075cbd0b853c4670e94f6ae4fe48d53b52b5e819`,
`src/wiz8/local_code/Health Stamina Mana.cpp::CharacterActionFatigueCost` and
`Combat Attack.cpp` around line 4338; `whizzardry-8` at
`31a3dfd57ee99fec8ff4b47f4b9be449c42de2fa`, the same cost function and
`src/wiz8/combat/attack.cpp` around line 3835. Both charge before decrementing
remaining swings and stop when the attacker is incapacitated. Tests exercise
cost ranges/disease and the actual round's stamina and exhaustion outcomes.

## Combat rest follow-up

Round completion ages conditions first, then restores stamina to accepted
participants who defended or whose worst remaining condition is sleep or
unconsciousness. Paralysis alone does not qualify. Death and departure from the
roster prevent recovery. Eligibility is assessed read-only; only actual vitals
changes borrow mutably. Accepted actor order still determines RNG consumption.

The Fighter bonus scales the rolled recovery by 3.3 times (60 + twice current
profession level) percent through level twenty, then stays at 3.3. Conversion to
an integer truncates after multiplication. Total character level and former
Fighter levels do not activate or strengthen another current profession's trait.
The existing Vitals operation caps stamina and wakes exhaustion above nine;
sleep itself is not cured by receiving stamina.

Evidence at the source revisions recorded above: decomp `local_code/Combat.cpp`
lines 1890–1910 and `character_skills.cpp::ScaleValueByProfessionLevel`;
Whizzardry `combat/fight.cpp` lines 1283–1304 and
`rules/character_derivation.cpp::ScaleValueByProfessionLevel`. Both agree.
This stays within the existing synchronous round operation and profile/vitals
ownership; no recovery queue or additional lifecycle state is needed.

Monster fatigue remains deferred until its recovery lifecycle is implemented:
merely charging attacks could strand an exhausted monster after a fight ends.

## Main menu and party creation

Interactive startup now opens a main menu instead of spawning the developer
party. New Adventure opens a six-slot party assembly screen, with a retained
four-page editor: identity/portrait, attributes, training and review. Accepted
character drafts can be revised or removed before beginning the campaign.
Canceling an edit preserves its previous accepted draft; leaving creation
removes the entire proposed party without changing the campaign.

`Session` owns the main-menu/creation/playing transition independently of the
in-game `Screen`. The creation root owns ordinary draft values and scoped UI
entities. Gameplay's `CharacterDraft` checks race/profession eligibility, the
60-point attribute pool after profession minimums, allocation ceilings, the
15-point training pool, five points per skill, and profession skill eligibility.
Changing the recipe explicitly refunds allocations. Attribute changes recompute
initial skill seeds. The synchronous `begin_adventure` operation validates the
whole party before spawning any actor, commits the actual completed profiles,
then requests the initial level. Vitals and starting equipment derive from those
profiles. No UI is needed to invoke this operation. UI edits consume no gameplay
randomness. Text and controls remain alive across page/value changes. Buttons use native
Bevy activation and tab navigation; name entry reads the native focus owner.

The frontend deliberately uses native text and a new layout rather than copying
retail's fixed widget hierarchy. Portraits use the retail catalog. Names support
Unicode, with a 24-character frontend limit. Unspent points are allowed and
explicitly shown at review. Race/profession arrows skip unavailable choices;
Valkyrie requires choosing female first. `--party` explicitly bypasses menus;
headless and screenshot runs retain the developer party default.

Evidence: both reference revisions listed in the combat follow-ups above,
`local_screens/MainMenuScreen.cpp`, `PartySelectionScreen.cpp`,
`CharacterScreen.cpp`, `local_code/CharGeneration.cpp`, and
`character_skills.cpp`; Whizzardry moves the availability table to
`src/data/character_tables.cpp`. The two availability tables agree. Both creation
implementations reduce the attribute pool for profession minimums **before**
calculating the per-attribute allocation cap. This corrects the older research
note's unconditional twenty-point cap. Small pools allow up to three points per
attribute, bounded by the pool itself.

Still absent: saved-game loading, loose-character file import/export, difficulty
and gameplay options, the intro movie, menu music, personality/voice selection,
and learned-spell selection. Casters begin without spells; this limitation is
shown at review. No claim of a complete retail character generator is made.
Created drafts are session-local and are not saved between launches. Retail
portraits and entering the monastery with a created party require an installed
copy and remain unverified in this environment.

Validation: 122 library tests passed, with 20 retail-dependent tests ignored.
Four new contract tests cover budgets/refunds, training eligibility and limits,
identity validation, and whole-party acceptance with preserved allocations and
single startup. An ephemeral offscreen harness rendered the menu and all five
creation views using Mesa software Vulkan at 1280x960, entered a name through
keyboard input, accepted its draft into the party, and left/re-entered creation
without spawning a campaign. Screenshots were inspected; unsupported default-font
glyphs were replaced. The harness was not added as a widget-layout regression
test. Retail portrait loading and playable level entry were not exercised.

## October 26 merge review

The world-preparation, ambient-lifetime, combat-fatigue, combat-rest and frontend
changes were reconciled with main's newer gameplay lifecycle. Round completion
keeps `RoundPhase`; menu acceptance carries the selected entrance/placement on
its destination, without restoring the removed global `PartyStart` resource.
The acceptance contract now checks that the requested entrance is preserved.
The current-frame picking/sky ordering, headless asset boundary and fixed-step
camp progression are retained.

Pending looping audio now updates its initial volume when the listener moves
before decoding completes. The audio-boundary check covers the requested gain;
it does not claim audible verification. Recognition coverage checks that aging
removes blindness and permits identification in the same operation, instead of
replaying private RNG draws. Character-creation test setup was tightened to pass
strict Clippy without warning suppressions.

Integrated validation: 151 library and 15 lifecycle tests passed. The 20
retail-dependent library tests and four retail scenarios remain ignored; the
scenario target compiles. Strict Clippy across all targets, package formatting
and whitespace checks pass. No retail gameplay, graphical or audible validation
was repeated on this integrated tree; earlier frontend screenshots are evidence
for that earlier revision only. These are local checks, not CI results.

Automap uses Tab/Shift+Tab to navigate controls while open; Escape closes it.
Tab still opens it from the world. This intentionally replaces the old Tab-to-close
binding so native keyboard control navigation is available.

Name and map-note editing use Bevy native text input (including selection and IME).
Map notes retain retail bitmap glyphs; their input filter still limits committed
text to that font’s supported characters and the retail note length. OS clipboard
and platform IME verification require an interactive display.
