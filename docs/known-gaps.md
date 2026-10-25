# Known gaps

What each implemented feature does, what it leaves out, and where it
departs from retail, in rough order of the
[implementation plan](implementation-plan.md)'s first slice. Keep an entry
next to its feature when that feature changes.

- Still open from the architecture review (the rest is done: level
  activation and travel, time advancement, level records, roster
  authority, inventory validation, gameplay-owned party freedom, sky and
  level visuals in presentation, bitmap fonts in content): navigation's asset module decodes PVL/OCT
  itself; the navmesh cache's hand-written RZNM codec could be a standard one;
  `EquipClass::ALL`-style mirrored enum arrays; the inventory screen's
  redraw snapshot does not cover all it draws (usability) and the held
  item's picture is not screen-owned; `LevelTriggers::with` regathers
  every prop each run and trigger actions are raw numbers with a door's
  lock read from its definition; several screens read Escape themselves.

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
  phases (party first on ties, then slot, then monster order), every member
  attacks the nearest monster in reach (slots 0-2 front, 3-5 rear), and
  monsters pick uniformly among the (attack, mode, member) choices that
  reach. Swings use retail's to-hit and penetration roll, hit locations,
  armour and damage reduction; death bookkeeping happens at the blow
  (experience pool, gold, out of the turn order) and the dead play their
  death cycle. A round is 120 game seconds and the clock is otherwise held;
  the fight ends when no hostile is left or after two rounds without an
  attack, sharing experience among the living. Fights are phased by default as in
  retail: the party's orders (Attack or Defend; A and D) stand, and each
  round waits for Enter or the round button; backslash switches to
  continuous. Defending gives +2 armour and stamina back at the round's end. Not yet: party movement and
  orders, spells, fumbles, criticals, effects, the off hand, monster
  movement and fleeing, factions, item drops, levelling.
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
  panel, buttons, scrolling and the PCX cell texture are still to come.
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
  what its triggers keep (switches, positions, contents, fountains,
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
