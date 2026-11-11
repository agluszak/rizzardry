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

1. **The Dark Savant's death** (`HandleScriptedNpcDeath`, `NPC Scripting
   Facts.cpp`). When species 0x234 dies, Vi (if she leads) and one other member
   speak event 24 and everyone event 28; the endgame turns to the book (or
   Phoonzang's notice comes); every hostile monster dies. Nothing happens here.
2. **Plot items taken** (`UpdateFactsAfterAcquiringItem`, `PC Item.cpp`). The
   Destinae Dominus 0x243, Astral Dominae 0x242 and Chaos Moliri 0x244 set
   `FACT_DESTINAE_POSSESS`, `FACT_ASTRAL_POSSESS`, `FACT_CHAOS_POSSESS` and clear
   their quests (`FACT_QUEST_GET_DD`, `_AD`, `_CM`); the Chaos Moliri also starts
   fact 0xb8's clock. The fake Astral Dominae 0x264 sets
   `FACT_ASTRAL_FAKE_POSSESS` and `FACT_ASTRAL_FAKE_POSSESS1`. Items 0x201,
   0x27c and 0x1d2 clear `FACT_QUEST_BRAFFIT_CIERDAN`,
   `FACT_QUEST_MYLES_FIND_DIAMOND` and `FACT_QUEST_MYLES_WEAPONS_CACHE`. None of
   these is set here, though `assets/tables/facts.ron` reads the three
   `*_POSSESS` facts, and so do the harassing encounter generators
   (`Generator::harasses`), which therefore never call anything up.
3. **Story monsters killed** (`MonsterKilled`). Brekek 0x131 sets
   `FACT_CROCK_BREKEK_DEAD`; the Rattkin queen 0x1a9 sets `FACT_QUEEN_IS_DEAD`
   and shifts the factions (2, 1, 0x10, 0x14); the second rat breeder (0x14f or
   0xdd) killed sets `FACT_TRYNNIE_KILL_RATS_DONE` and puts faction 7 in band 0;
   Al-Sedexus 0x22b sets `FACT_RAPAX_ALSEDEXUS_DEAD`, clears
   `FACT_QUEST_KILL_ALSEDEXUS` after `FACT_ALSEDEXUS_ATTACK`, and frees the
   chosen member from infatuation with event 37; the king 0x181 sets
   `FACT_KING_IS_DEAD`; the Lava Lord (0x175 or 0x222) sets `FACT_LAVALORD_DIE`.
   None is set here.
4. **Companions and regions** (`NPC Manager.cpp`). A companion entering a
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
5. **NPC records** (`gameplay_databases.h:302`): an NPC with an allied
   faction is friendly (50) while a companion of that faction is in the front
   two seats (`GetNpcDisposition`); a hostile NPC whose record asks for it
   says its combat line as the fight opens (`combat_script_notice_enabled`,
   `Combat.cpp:147`); a price an NPC asks for a quote is lowered by the charm
   on them (percent) and by the party's best Communication (a five-hundredth
   per point), never under 1 (`NPCInteractionSubscreen.cpp:4283`).
   `retail/npcs.rs` reads neither field and the price is asked as written.
6. **Theft** (`ScoreNpcTheft`, `AttemptNpcItemTheft`): pickpocketing in
   conversation and stealing an offered item in trade, scored 0 to 127
   against d100 from Stealth (Rogues add a share of Dexterity), a quarter of
   the disposition swing and a random penalty; the item is taken (and the skill
   practised), refused, or the thief is caught.
7. **Infatuation and the Destinae Dominus** (`GameTurnsPassedChar`).
   Infatuated outside the Rapax levels (bands 9 and 10), a member takes one
   damage a minute at one in four and is hexed without end; inside, that hex
   lifts. Carrying the Destinae Dominus (not in hand) without the helm 0x239 on
   the same member drives them insane without end; with it, that insanity lifts.

## Not here: rules of play

1. **Monsters never recover over time and poison never hurts them out of a
   round** (`AgeMonsterSight`, `RebuildMonsterRegenRates`): the species'
   health and stamina regeneration a minute, the pooled rate at a half outside
   a fight and in full while the party camps, a quarter while diseased, and
   damage a minute (poison) half again in a fight. Here only the party's
   members recover and are poisoned between rounds.
2. **What else changes recovery** (`GameTurnsPassedChar`,
   `RebuildCharacterRegenRates`, `Gameplay Mods.cpp`): items' regeneration
   bonuses (read into `Protection::regeneration`, never applied: a point a
   minute up, or down as damage, fatigue and drain); a half again on every rate
   while a member with Camp Recovery is awake and unafflicted, or under the
   effects that boost it; Health Regeneration's trait (health scale times
   `ScaleValueByProfessionLevel(16.67)`, half that when the pace gives none); the
   Faerie's spell recovery times 1.25 and the Lizardman's times 3/4; disease
   quartering health recovery; being swallowed, −1 − level/2 of the swallower
   a minute (−5 if it is gone).
3. **Monsters tire as they act** (`MonsterActionFatigueCost`): advancing or
   backing off 5 + below(5), approaching or returning 3 + below(3), waiting
   1 + below(2), attacking 2 + below(3) or 5 + below(8) for detail 3; twice that
   diseased. Here a monster spends no stamina.
4. **Pray and Turn Undead** (`Combat Hostility.cpp`). Turn Undead: a Bishop's or
   Priest's once a fight, while an undead hostile is in it; Dispel Undead at
   (Bishop + Priest levels) shared among the undead, 1 to 7, dropping undead at
   random when the power is less than their number. Pray: trait 11's once a
   fight, a weighted table (`CharacterPrayAction`, 342 lines). Neither is an
   order here. (Protect is on [attack](rules/attack.md).)
5. **Changing equipment in a fight** needs the Equip order, which spends the
   member's turn (`W8_ACTION_EQUIP`, `Combat.cpp:1332`, the refusals 0x902 to
   0x904 in `ReviewCharacterScreen.cpp:2420`). Here a member re-equips freely
   in a fight.
6. **Under water** (`UpdateCampFatigue`, run while the camera sways): every
   2.5 s each member able to act, not an Android and without the wetsuit 0x1e5,
   rolls (ticks)d(count/6 + 2), count being the ticks spent under; stamina takes
   it first and the rest is damage; notice 0x1da once; while so tired, stamina
   does not come back without the wetsuit. Here water only quenches Fire.
7. **Hard landings** (`HandleLevelOverride`): a fall whose vertical motion
   (velocity over 3000) passes 0.35 after it was under 0.1 deals
   `pow(fall + 0.7, 8)` d6 to every member, with notice 0x252 and the fall sound.
8. **An item that recharges**: 0x266 gains a use every game hour wherever the
   party carries it.
9. **The Alchemist brews** a potion while camping when the brew's cooldown has
   run out (`BrewAlchemistPotion`, Make Potions).
10. **Buying starting gear**: the character screen offers a new character
    their profession's starting equipment for its price when the party's gold
    covers it (`CharacterScreen.cpp:640`, `ComputeStartingEquipmentCost`). When
    the offer is made, and whether gear is otherwise free, was not settled; here
    starting gear is always given.
11. **Swallowing** (`W8_ATTACK_EFFECT_SWALLOW`, `Combat Attack.cpp:3006`): a
    monster's blow swallows a member (Missing without end, saved against with
    Divine) and its group retires from the encounter budget; on a monster it
    kills. The swallowed member loses health over time (item 2 above), recovers
    no stamina, and is let out when the swallower dies
    (`ReleaseMonsterConditionBindings`). `assets/tables/weapon_effects.ron` has
    no row for it, so the effect never lands.
12. **A group's new leader** (`ElectGroupLeaderMember`, called as the leader
    dies, `MonsterManager.cpp:1188`): the member of highest level (ties to the
    one placed last) that is not Turncoat or worse leads, taking over the dead
    leader's script and the noise it heard; allied groups elect a leading
    group the same way. Here the group's first member stays its leader dead,
    its script dies with it, and the rules that ask for the leader read a body.
13. **Who answers for a kill** (`RecordMonsterKill`): the faction takes
    offence only when a member or a friendly monster killed. Here every
    death that was not summoned counts. The kill is announced only if the
    party had seen the monster. Killing species 0x146 costs its faction 40
    (`RecordFactionOffense`); here it counts as any other.
14. **Kills counted**: each member's kills, shown on the character sheet
    (`ReviewCharacterScreen.cpp:1026`), and each level's; a member who kills a
    monster flagged significant whose level is at least the party's average
    plus two remarks (event 25). None here.
15. **Armour from Stealth and Reflextion** (`CalcArmorClasses`,
    `GameplayCode.cpp:840`): Stealth adds a tenth of its level, Reflextion
    (if known) a twentieth plus one. Neither is in `actors/arms.rs` `armour`.
    The original also drops race, speed, skill, spell and load parts of
    armour from a member worse than unconscious.
16. **Training by being attacked** (`MarkDefenderSkillUse`,
    `PracticeCombatRoundSkills`): a member attacked in a round practises
    Stealth, Reflextion, and Shield (with a shield worn, not caught unaware,
    not by a natural attack), two points each at the round's end. Here only
    Iron Skin trains from blows taken.
17. **Starting and ending a fight** (`ToggleCombatMode`): the player may
    start a fight, and end one when no group in it can still engage, nothing
    is in flight and nobody is acting (notices for each refusal). Here fights
    begin and end only by the rules.
18. **The party wiped out** (`BeginPartyDeath`, `MainGameScreen.cpp:3168`): when
    nobody alive is above Dead, the death sound and the `CombatLose` music
    play and the screen fades to the party-death screen; an Iron Man game
    deletes its saves. Here nothing happens when the whole party dies.
19. **What conditions do to figures** (`ApplyConditionModifiers`, `Gameplay
    Mods.cpp:375`). Irritated −2 to hit and −2 armour; Nauseated −5 and −4;
    Afraid −3 and −2; Poisoned −2 and −2; Hexed −5 to hit and −20 to every
    attribute and skill; Slowed −50 Speed; Insane −50 Intelligence; Blind −50
    Senses (a blind fighter instead gains `ScaleValueByProfessionLevel(50)` −
    50); Webbed −50 Dexterity. Blind (without the trait), Webbed, Asleep,
    Paralyzed and Unconscious leave the body out of formation, its armour no
    better than −5. The same block is folded into monsters
    (`ConvertMonsterAttributes`). Here no condition changes anyone's figures:
    `actors/stats.rs` `refit` reads none.
20. **What worn items add** (`AccumulateEquipmentModifiers`): each item may
    raise one skill and one attribute (record bytes 0xb1 to 0xb4, which
    `retail/items.rs` does not read), and an item worn outside the hands adds
    its to-hit and damage bonuses. Here only the weapon in hand adds them.
21. **Monsters that open doors** (`can_open_doors`, `MonsterManager.cpp:255`):
    such a monster's paths go through closed doors. Here every walker needs a
    door open.
22. **Monsters striking several** (`attack_multiple_targets`, `Combat
    Attack.cpp:1954`): within short range such a monster picks a target anew
    for each swing. Here every swing of an attack goes to one target.
23. **A target picked again** (`RepickActionTarget`, `Combat Attack.cpp:2397`):
    each time a member's target has to be picked anew in a round, their aim
    loses `50 − (Speed − 25 × times − 50 + attack score)^¾`, never below
    nothing.
24. **The Bishop lifts curses' hold** (`RevealCharacterItemBindingsByProfession`,
    `RCSItemsPage.cpp:332`): from the character screen one with Remove Cursed
    Items reveals the curses on a member's worn items whose identify
    difficulty is at most (their profession level / 4 + 1) × 3, so they can be
    taken off; all, some or none, said with a sound. The trait is read by
    nothing here.
25. **Waking to an ambush** (`ResolveSurpriseWake`): when a camping party is
    surprised, each member rolls below(100) − 20 − 70 percent of Senses; above
    nothing they are Asleep for roll / 30 + 1 rounds, otherwise they are told
    they woke. Here the camp's surprise is rolled but everyone wakes at once.
26. **Encounter scripts** (`MonGen::GenerateEncounter`): each entry of an
    encounter table names the script its group's leader runs (`Default.MSF`
    when empty). `retail/encounters.rs` reads past the names, so every
    encounter runs the default.
27. **Arranging the formation** (`MGSFormation.cpp`, `ReconcilePartyFormation`):
    the player drags members between the five rows on the formation panel; in
    a fight the change takes effect as the round ends (notice 0x7d9), and the
    formation the fight began with is put back when it ends
    (`RestoreCombatFormation`). Here the board only shows the formation and
    members are seated by the party's order.
28. **Secrets found by searching** (`Search.cpp`, `RegisterSearchableTrigger`):
    a trigger flagged searchable (`retail/trigger.rs` reads past the flag) is
    found like a hidden item and then runs, with its message from the level's
    message file. A find also trains the finder's Scouting (5 points), lets the
    party try to identify a found item, brings a remark (two events for items,
    event 16 for secrets) and turns the camera to it; in search mode, one pulse
    in a hundred that finds nothing brings a remark. Here only hidden items are
    found, with a notice and nothing else.
29. **Places in a level** (`CUBE` chunk of the level's `.sts`,
    `MasterFunctionList.cpp` `g_world_cursor_node_handlers`): boxes the party
    walks into. By type: a message from the level's message file shown once
    (some only while searching, to a Ranger, or under Detect Secrets), a
    message on demand, a party remark, the Ascension Peak bodies (with all
    three relics and either meeting assigned: a remark, `AP_SeenBodies`, and
    `FACT_QUEST_PEACE_GOTO_AP` cleared), and a male member's remark while
    Al-Sedexus holds one. `retail/population.rs` reads the `LOCK` and `MONG`
    chunks but not `CUBE`, so none of these exists here.
30. **The camp's two faces** (`NormalizeMasterFunctionValue`,
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
    the party's (`PartyDead`), a spell running out (`Spell Expiry`) and others
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
