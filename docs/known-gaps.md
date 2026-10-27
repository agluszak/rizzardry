# Known gaps

Only unfinished behavior, deliberate departures and validation limits belong here.
See [research](research/README.md) for recovered mechanics and
[priorities](implementation-plan.md) for next outcomes. This is not a changelog.

## UI and character creation

- Character review lacks the learned-spell list, level-up editing, trait/enchantment
  details, several combat/armor values and the improved-skill palette. See the
  [original-game comparison](research/character-review.md) for exact limits.
- The main menu and party assembly use retail artwork; the character editor still
  has a custom layout. Candidate review and persistence are
  absent. Saved-game loading, character import/export, difficulty/gameplay options,
  intro movie, menu music and personality/voice are also absent. First-spell
  picks use a plain list; candidates remain session-local.
- Several HUD bank actions remain unimplemented. Inventory lacks stack splitting,
  ammunition pouring, weapon-partner stowing, remaining item actions and filtering.
  Containers/bags currently transfer contents directly instead of opening a picker.
- Automap explored cells and minimap note projection are absent. Notes survive
  travel within the session but cannot yet be saved to disk.
- Keyboard navigation intentionally differs: Tab opens the map from the world,
  then navigates controls while it is open; Escape closes it. F6 enters HUD focus,
  Tab navigates it and Escape returns to world input.
- Map notes use native text editing with retail bitmap presentation and supported
  glyph/length limits. Platform IME and OS clipboard still need interactive checks.

## Combat and progression

- Additional orders, criticals/fumbles, off-hand attacks, levelling, fleeing and
  monster spellcasting remain. The party's combat move pauses the round instead
  of interleaving monsters by distance covered; faction standings change only by
  kills, not by dialogue, facts or theft.
- Magic: buffs (enchantments and effect slots such as Bless, Haste, Armorplate),
  continuing clouds, summoning, Turncoat, Charm and utility spells (light,
  portals, identify, locks) are refused before anything is spent. Monster and
  weapon attack effects (poison and its strength among them) are not ported.
  Effects that retail applies when their visuals end apply at the action, since
  gameplay does not wait for artwork. Spell missiles fly at 15 m/s and hit their
  captured victim. The spell views pick enemies by retail's auto-target rule,
  not by pointing in the 3D view; they neither scroll nor list the party pool.
  Difficulty has no options screen yet.
- Party swing fatigue/rest are implemented; shared-load fatigue scaling, Berserk
  order and monster fatigue/recovery are not. Charging monster fatigue alone could
  leave exhausted monsters stranded after combat.
- Loot appears at the lethal blow rather than the dying animation; NPC monster
  item lists are not ported. Recognition uses view/far-clip tests instead of the
  retail drawn-actor test, and Mythology practice is absent.
- Camping lacks ambushes, waking Senses checks and music.

## World interaction and persistence

- Keyless locked doors await lock/trap interaction. Missing loot table IDs in two
  retail containers produce nothing, matching retail rather than invented content.
- Search lacks searchable triggers, practice, turning toward a find, the identify
  attempt and the rare mishap. Item throws use a pointer ray rather than retail's
  throw angles. Plane-shaped fall triggers are not decoded as places.
- Lifts, random teleports and some level-specific camera completion reactions are
  absent. Arrival does not probe ground; the party physically settles afterward.
- Many trigger actions remain, including lights, missiles, timed/required-item
  actions and spell fountains. Health/stamina fountains already work.
- Dormant records retain map state during this session. Disk saves/load,
  dormant-level time catch-up and complete RPC/story/crafting flows remain.

## Rendering and evidence

- Monster static lights use centre falloff rather than per-vertex N·L; attenuation
  is not confirmed. Only nearest LOD is used; particles, sounds and skins are absent.
- The screenshot tool intermittently omits unchanged UI artwork. Successful map
  assertions and some complete frames do not establish that this is resolved.
- Headless gameplay has no renderer/input/audio dependencies at runtime, but shares
  the compiled package; it is not a separate Cargo feature build.
- Original-executable character comparison covers the documented pre-game state,
  not complete in-game parity. Headless checks do not prove retail gameplay,
  graphical or audible equivalence. Record exact validation in PRs, not dated
  test-count diaries in this file.
