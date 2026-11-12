# What of the original is not here

A survey of the decompilation (`../wizardry-8-decomp/src/wiz8`) for what the
game does that this one does not. Rule pages under [rules/](rules/) keep their
own lists of defects and missing rules; this page points at those and lists
what no rule page covers.

## How it was done

Every one of the decompilation's 212 source files was inventoried: 4,644
routines, each with the comment the decompilation gives it. 532 of them are
named somewhere in `src/`, `docs/` or `assets/tables/`. A name is weak
evidence either way: this code names a retail routine only beside the
formula it takes from it, and some names match by accident (`TurnUndead` is a
trait here, not the action). So the unnamed routines of every gameplay file
were judged by their comment, or by their name where the decompilation gives
none, and those that read as a rule of the game were looked up in this code
by what they do, and their bodies read where the answer was not plain. A
second pass went through the undescribed routines of the gameplay files and
the screens' routines by name. A third went system by system: every skill,
condition, attribute, spell, party effect and attack effect of the original,
and every field of its item, monster and NPC records, each looked for where
the original reads it and where this code does.

A fourth pass went subsystem by subsystem through what the first three
touched only lightly: search, the journal, the automap, time of day, the main
screen's panels, the character, trade and options screens, the front end,
what combat shows in the world, and the level-specific code (its load-time
setups, the places in a level, the camp's two faces).

Not done: reading every body. The engine files were judged by their comments
where the comments say they are rendering, collision, pathing or file work.
The item-information and spell-information dialogs were not compared line by
line. Monster behaviour, the fight and casting rest on the audits recorded on
their rule pages; this survey did not redo them. The level devices rest on
[devices](rules/devices.md), which lists what differs.

## Left out on purpose

- **The engine's own machinery**: the octree, its paths and collision
  (`Oct*.cpp`, `GameData.cpp`'s movement, `Navigator.cpp`, `PathAI.cpp`), the
  renderer and its meshes, textures, lights, particles and cameras (`3d*.cpp`,
  `Video2.cpp`, `st*.cpp`, `materials.cpp`, `AniMesh.cpp`, `AnimObj.cpp`,
  `GrCycle.cpp`), and its lists (`PList.cpp`, `IList.cpp`, `BitArray.cpp`).
  Bevy, Avian and the navigation mesh do this work here.
- **File formats and loading** (`chunk.cpp`, `GDFileIO.cpp`, `LevelFile.cpp`,
  `ReadLevel.cpp`, `ReadMesh.cpp`, `GameplayDatabase.cpp`, `Gameplay Init.cpp`):
  `src/retail` reads the retail files instead.
- **The interface toolkit** (`Controls.cpp`, `RegionManager.cpp`,
  `Text_Input.cpp`, `DialogBase.cpp`, `stButton.cpp`, `stListBox.cpp`,
  `DialogScrollBar.cpp`, `VideoObjectManager.cpp`): Bevy's UI.
- **Developers' tools**: the monster and item placement editor (`mipe.cpp`,
  `mipeEdit.cpp`), the notes recorder (`Traps.cpp`'s record mode), the quote
  audit (`AuditNpcScriptQuotes`), the CD lookup (`FindGameDataPath`).

## Known gaps kept on the rule pages

- [attack](rules/attack.md) section 9: a waiting monster never notices an
  attack; members cannot Protect.
- [casting](rules/casting.md) section "How this is built": practice from
  fizzles, the camp mode, Remove Curse's power, spells on monsters not at war,
  and the unsettled questions.
- [monsters](rules/monsters.md) "Defects" and "Missing": the engagement
  state, a ward out of formation, monsters stopping short of the party, the
  30-tick wait after a noise.
- [devices](rules/devices.md) section 4: each level device that differs.
- [attack](rules/attack.md) section 7: the quiet the party keeps during
  Al-Sedexus's capture.

## Not here: the story

These stop or change the story.

1. **Companions and regions** (`NPC Manager.cpp`). A companion entering a
   region they serve is bound: −20 to every attribute and skill while bound,
   and remark 0x56. Rodan without Drazic (event 0x6c) and Drazic without Rodan
   (0x67) leave outside region 13. A companion who serves the destination
   region refuses to go there (`QueueNpcTravelRefusals`,
   `QueueNpcDepartureEvents`). A companion whose own faction (or, for Sparkle,
   faction 4) is among the hostile groups of a fight leaves it (notice 0x272,
   `QueueNpcCombatScript`). At a fight's end a companion with healer or item
   assist helps when a member died (`ApplyCombatEndEffects`). Here a companion
   only declines to join where they keep a post. An NPC's body dying marks the
   NPC as having appeared, except Vi's before her rescue is asked for
   (`HandleScriptedNpcDeath`); not checked here.
2. **NPC records** (`gameplay_databases.h:302`): an NPC with an allied
   faction is friendly (50) while a companion of that faction is in the front
   two seats (`GetNpcDisposition`); a hostile NPC whose record asks for it
   says its combat line as the fight opens (`combat_script_notice_enabled`,
   `Combat.cpp:147`); a price an NPC asks for a quote is lowered by the charm
   on them (percent) and by the party's best Communication (a five-hundredth
   per point), never under 1 (`NPCInteractionSubscreen.cpp:4283`).
   `retail/npcs.rs` reads neither field and the price is asked as written.
3. **Theft** (`ScoreNpcTheft`, `AttemptNpcItemTheft`): pickpocketing in
   conversation and stealing an offered item in trade, scored 0 to 127
   against d100 from Stealth (Rogues add a share of Dexterity), a quarter of
   the disposition swing and a random penalty; the item is taken (and the skill
   practised), refused, or the thief is caught.
4. **Infatuation and the Destinae Dominus** (`GameTurnsPassedChar`).
   Infatuated outside the Rapax levels (bands 9 and 10), a member takes one
   damage a minute at one in four and is hexed without end; inside, that hex
   lifts. Carrying the Destinae Dominus (not in hand) without the helm 0x239 on
   the same member drives them insane without end; with it, that insanity lifts.

## Not here: rules of play

1. **Pray and Turn Undead** (`Combat Hostility.cpp`). Turn Undead: a Bishop's or
   Priest's once a fight, while an undead hostile is in it; Dispel Undead at
   (Bishop + Priest levels) shared among the undead, 1 to 7, dropping undead at
   random when the power is less than their number. Pray: trait 11's once a
   fight, a weighted table (`CharacterPrayAction`, 342 lines). Neither is an
   order here. (Protect is on [attack](rules/attack.md).)
2. **Changing equipment in a fight** needs the Equip order, which spends the
   member's turn (`W8_ACTION_EQUIP`, `Combat.cpp:1332`, the refusals 0x902 to
   0x904 in `ReviewCharacterScreen.cpp:2420`). Here a member re-equips freely
   in a fight.
3. **Under water** (`UpdateCampFatigue`, run while the camera sways): every
   2.5 s each member able to act, not an Android and without the wetsuit 0x1e5,
   rolls (ticks)d(count/6 + 2), count being the ticks spent under; stamina takes
   it first and the rest is damage; notice 0x1da once; while so tired, stamina
   does not come back without the wetsuit. Here water only quenches Fire.
4. **Hard landings** (`HandleLevelOverride`): a fall whose vertical motion
   (velocity over 3000) passes 0.35 after it was under 0.1 deals
   `pow(fall + 0.7, 8)` d6 to every member, with notice 0x252 and the fall sound.
5. **An item that recharges**: 0x266 gains a use every game hour wherever the
   party carries it.
6. **The Alchemist brews** a potion while camping when the brew's cooldown has
   run out (`BrewAlchemistPotion`, Make Potions).
7. **Buying starting gear**: the character screen offers a new character
    their profession's starting equipment for its price when the party's gold
    covers it (`CharacterScreen.cpp:640`, `ComputeStartingEquipmentCost`). When
    the offer is made, and whether gear is otherwise free, was not settled; here
    starting gear is always given.
8. **A swallower's group** (`BindMonsterToCharacterDependence`): the group
    of a monster that swallows a member retires from its encounter's budget.
9. **A leading group** (`ElectAlliedLeaderGroup`): when an encounter's
    leading group is gone, its companion groups elect a new one among
    themselves, by their members' highest level. Here the companions keep to
    the group they came with.
10. **A kill unseen** (`RecordMonsterKill`): a death is announced only if
    the party had seen the monster. Here every death in a fight is announced.
11. **Kills counted by level** (`monster_kill_count`): each level keeps how
    many hostiles died in it. Nothing reads it here, and it is not kept.
12. **Training by being attacked** (`MarkDefenderSkillUse`,
    `PracticeCombatRoundSkills`): a member attacked in a round practises
    Stealth, Reflextion, and Shield (with a shield worn, not caught unaware,
    not by a natural attack), two points each at the round's end. Here only
    Iron Skin trains from blows taken.
13. **Starting and ending a fight** (`ToggleCombatMode`): the player may
    start a fight, and end one when no group in it can still engage, nothing
    is in flight and nobody is acting (notices for each refusal). Here fights
    begin and end only by the rules.
14. **Iron Man** (`BeginPartyDeath`): an Iron Man game deletes its saves when
    the party falls. There are no saves here yet.
15. **What worn items add** (`AccumulateEquipmentModifiers`): each item may
    raise one skill and one attribute (record bytes 0xb1 to 0xb4, which
    `retail/items.rs` does not read), and an item worn outside the hands adds
    its to-hit and damage bonuses. Here only the weapon in hand adds them.
16. **Monsters that open doors** (`can_open_doors`, `MonsterManager.cpp:255`):
    such a monster's paths go through closed doors. Here every walker needs a
    door open.
17. **Monsters striking several** (`attack_multiple_targets`, `Combat
    Attack.cpp:1954`): within short range such a monster picks a target anew
    for each swing. Here every swing of an attack goes to one target.
18. **A target picked again** (`RepickActionTarget`, `Combat Attack.cpp:2397`):
    each time a member's target has to be picked anew in a round, their aim
    loses `50 − (Speed − 25 × times − 50 + attack score)^¾`, never below
    nothing.
19. **The Bishop lifts curses' hold** (`RevealCharacterItemBindingsByProfession`,
    `RCSItemsPage.cpp:332`): from the character screen one with Remove Cursed
    Items reveals the curses on a member's worn items whose identify
    difficulty is at most (their profession level / 4 + 1) × 3, so they can be
    taken off; all, some or none, said with a sound. The trait is read by
    nothing here.
20. **Waking to an ambush** (`ResolveSurpriseWake`): when a camping party is
    surprised, each member rolls below(100) − 20 − 70 percent of Senses; above
    nothing they are Asleep for roll / 30 + 1 rounds, otherwise they are told
    they woke. Here the camp's surprise is rolled but everyone wakes at once.
21. **Arranging the formation** (`MGSFormation.cpp`, `ReconcilePartyFormation`):
    the player drags members between the five rows on the formation panel; in
    a fight the change takes effect as the round ends (notice 0x7d9), and the
    formation the fight began with is put back when it ends
    (`RestoreCombatFormation`). Here the board only shows the formation and
    members are seated by the party's order.
22. **Secrets found by searching** (`Search.cpp`, `RegisterSearchableTrigger`):
    a trigger flagged searchable (`retail/trigger.rs` reads past the flag) is
    found like a hidden item and then runs, with its message from the level's
    message file. A find also trains the finder's Scouting (5 points), lets the
    party try to identify a found item, brings a remark (two events for items,
    event 16 for secrets) and turns the camera to it; in search mode, one pulse
    in a hundred that finds nothing brings a remark. Here only hidden items are
    found, with a notice and nothing else.
23. **Places in a level** (`CUBE` chunk of the level's `.sts`,
    `MasterFunctionList.cpp` `g_world_cursor_node_handlers`): boxes the party
    walks into. By type: a message from the level's message file shown once
    (some only while searching, to a Ranger, or under Detect Secrets), a
    message on demand, a party remark, the Ascension Peak bodies (with all
    three relics and either meeting assigned: a remark, `AP_SeenBodies`, and
    `FACT_QUEST_PEACE_GOTO_AP` cleared), and a male member's remark while
    Al-Sedexus holds one. `retail/population.rs` reads the `LOCK` and `MONG`
    chunks but not `CUBE`, so none of these exists here.
24. **The camp's two faces** (`NormalizeMasterFunctionValue`,
    `LoadAwayCampChest`): travel to the camp (0x10, `CMP`) goes to the camp
    without the Rapax (0x26, `CNR`) unless the Rapax away camp exists (two or
    three relics held) or the party is at it; arriving at the Rapax camp moves
    what was left at the empty camp into its chest. Here `CMP` always loads the
    Rapax camp, and the chest holds nothing.

## Not here: the screens and what the world shows

1. **Combat in the world**: no missiles fly (arrows, thrown weapons, spell
   missiles; `Missile.cpp`), no spell or breath visuals play (`Spells.cpp`), and
   monsters play only their idle, walk, death and scripted cycles, never their
   attack, hit or casting cycles (`StartMonsterAttackCycle`). Nor do they show
   their wounds (`UpdateMonsterDamageAppearance`).
2. **The fight's monster list** (`RedrawCombatMonsterList`,
   `MonsterListRegionEvent`): the groups in the fight listed at the screen's
   left, hovered to highlight, clicked to aim, right-clicked for their notice.
3. **Monster information** (`OpenMonsterInfoDialog`, `MonsterInfoDialog.cpp`):
   right-clicking a monster opens its sheet, more of it the better the party's
   Mythology.
4. **The automap's markers** (`RenderAutomapMarkers`): monsters the party sees
   (by side) and items lying in the world are drawn on the map; and the map
   shows only what the party has explored (`LightAutomapCell`). Here it shows
   notes and the party over every walkable surface.
5. **The journal's notice** (`RecordFactChangeForJournal`): a fact the
   difficulty lets the player see says "journal updated" in blue, with its
   sound.
6. **Options** (`OptionsScreen.cpp`): combat speed, text delay, monster
   movement speed, camera rotation, mouselook and its smoothing, stopping for
   events, auto-targeted spells, auto-advancing the selection, swapping weapon
   sets automatically, simplified NPC talk, verbose combat messages, skill
   messages, numeric hit points, autoscrolling, tooltip delay, each sound
   channel's volume and mute, confirmations and subtitles, gamma and the
   graphics options, and rebinding keys. Here: difficulty, combat mode, one
   volume, invert mouse, fullscreen.
7. **Party creation's Review Character** does nothing
   (`presentation/frontend/party.rs`).
8. **Movies** (`IntroScreen.cpp`): the logos, the intro, and the dark ending's
   film, Bink videos in `Data/Flics`.

## Not here: what the party says and hears

1. **Idle chatter** (`ProcessFollowUpEvents`): every five to six minutes a
   member says event 14 or 15 and another answers two to eight seconds later.
2. **A fight's opening** (`EvaluateCombatDifficulty`): the party's strength
   (levels weighted over √(6 × members)) against the hostiles' experience
   (`EstimateCombatThreatLevel`) picks the music, `CombatEasy`, `Combat` or
   `CombatLousy`, at ±20 percent, and a member says event 18, 19 or 20 to match.
   Here the level's playlist plays on.
3. **A monster missing a member** (`Combat Attack.cpp:1791`): below half their
   health the member panics at 15 percent (event 22), otherwise half the time
   they say event 19, 15 or 20.
4. **A fumble** (`QueueFumbleReaction`): half the time the fumbler remarks,
   otherwise the one struck, or another member.
5. **Exceptional items** (`DeliverExceptionalItemReaction`): once per item, a
   remark on the relics, the helm and the fake; the diamond sets
   `FACT_DIAMOND_FOUND` and Myles remarks; any identified item the finder can
   use worth more than 500 times their level, message 6.
6. **Falling to nothing** (`MaybeStartIncapacitationEvent`): a member at no
   health may say one of three incapacitation events.
7. **Footsteps** (`PlayFootstep`, `AmbientSound.cpp`) by the surface and
   material walked on.
8. **A member gone missing, at a fight's end** (`ApplyCombatEndEffects`):
   another member remarks (effect 27) on each member who is missing and not
   dead.
9. **Healed before speaking** (`HealCharacter`): a member healed back above
   the thresholds loses the wound remarks still waiting (going down, crying
   out, and event 17). Here a waiting remark is said anyway.
10. **Ready to advance** (`RefreshLevelUpReadyNotices`): the first time a
    member has the experience for a level, a message box says so, they speak,
    and their portrait shows a level button. Here only the character screen
    offers the level.
11. **Fixed sounds never played here**: weapons readied as a fight starts
    (`Ready Weapons`, `StartCombat`), the chime at a round's end
    (`EndTurnChime`, `AdvanceCombatRound`), a member's death (`CharacterDead`),
    a spell running out (`Spell Expiry`) and others
    taking effect (`GeneralMagic`), a spell learned (`Spell Learned`), a level
    gained and points spent on the character screen (`GainLevel`, `Points
    Spent`), a journal entry (`Journal Entry`), an item that cannot be dropped
    (`CantDrop`), the movement bar appearing (`Movement_Bar_Pop_Up`), the
    item picker's beep (`beep2`), and the options' volume samples
    (`Interface Swoosh`, `Interface Vox`).

## Seen, not yet checked

- Bela's voice cue on frame 16 of NPC 0x1b6's cycle (`TriggerBelaVoice`).

## Not decided

- **Saving and loading** (`LoadSaveGame.cpp`, 44 routines, and each module's
  save and load).
- **Bringing a Wizardry 7 party in** (`Party Import.cpp`): characters,
  attributes, skills, spells and equipment converted, and the start chosen by
  the Wizardry 7 ending (`ChooseNewGameStartLocation`).
