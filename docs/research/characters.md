# Party characters: record, tables and derived stats

What a party member is, which ids item usability tests against, and how the
character sheet's numbers are derived. Paths are relative to
`/home/agluszak/code/personal/decomp/whizzardry-8` (commit `3d046808`).
Abbreviations:

- `ch.h` = `include/wiz8/rules/character.h`, `rec.h` = `include/wiz8/data/records.h`
- `sk.h` = `include/wiz8/data/skills.h`, `ct.cpp` = `src/data/character_tables.cpp`
- `cd.cpp` = `src/rules/character_derivation.cpp`, `cs.cpp` = `src/rules/character_stats.cpp`
- `ps.cpp` = `src/rules/party_stats.cpp`, `ir.cpp` = `src/rules/item_rules.cpp`
- `cg.cpp` = `src/wiz8/local_code/CharGeneration.cpp`, `gc.cpp` = `src/wiz8/local_code/GameplayCode.cpp`
- `csk.cpp` = `src/wiz8/character_skills.cpp`, `pri.cpp` = `src/wiz8/dialog_code/ProfRaceInfoDialog.cpp`
- `npc.cpp` = `src/wiz8/npc_creation.cpp`

String indices were decoded from retail `data/strings/stringdata.dat`. Item
statistics were counted over retail `data/databases/items.dbs` (819 records).
All behaviour comes from Whizzardry, not the original-binary decomp, so treat
it as reconstructed. Open questions are marked **Uncertain** and listed in §7.

Every race, profession, skill and trait table below is **hard-coded in the
executable** (Whizzardry copies the retail tables from `.rdata` addresses,
`ct.cpp:6-8`, `ct.cpp:69-73`). No database file holds them. The only character
data in `data/databases` is `npc.dbs`, which holds the recruitable NPCs'
character templates (`rec.h:270-287`).

## 1. The character record

The retail record is the runtime struct `W8Character` (`ch.h:123-316`), about
0x1862 bytes. A saved character is a whole record in `<name>.CHR`
(`ch.h:135`). Fields a rewrite needs, grouped by meaning:

| Group | Fields | Meaning | Source |
| --- | --- | --- | --- |
| Identity | `name` (UTF-16 in retail, at 0x05); `name_part_2` | Display name; the second part is only set for NPCs | `ch.h:135-137`, `npc.cpp:46-47` |
| Profession | `iProfession`, `original_profession` | Current profession and the one the character was created in. Both matter for HP and the level band | `ch.h:138-141` |
| Race, gender | `iRace`, `gender` | 0 male, 1 female; -1 (unset) only while being created | `ch.h:142-148`, `rec.h:74-79` |
| Portrait | `portrait_index` | 0..79, an index into the 80 large/medium/small portrait images (`data/portraits/{large,medium,small}/`, stems `hummf`, `elfff`, ...; order in `src/wiz8/local_code/CatalogArtwork.cpp:38-117`). The default comes from a sex×race×profession table | `ch.h:149-152`, `gc.cpp:283-289` |
| Personality, voice | `personality` 0..8, `voice` 0/1 | Pick the party-quote file `data/quotes/pcs/{m,f}_{aggr,intell,burly,chaos,cun,ecc,kind,laid,loner}{1,2}0.msg` (36 files) | `ch.h:156-157`, `gc.cpp:313-350` |
| Levels | `uiExpLevel`; `profession_levels[15]` | Total level; levels per profession. Both clamp to 50 before every derivation | `ch.h:158-160`, `cd.cpp:281-283` |
| Level band | `level_band`, `level_band_base` | 0..8 title band (Apprentice, Novice, ...) for the current profession | `ch.h:162-166`, `gc.cpp:255-281` |
| Experience | `experience`, `experience_goal`, `experience_previous_goal` | Ready to level when `experience >= experience_goal` | `ch.h:175-180`, `gc.cpp:222-237` |
| Tallies | `kill_count`, `death_count` | Shown on the camp stats page | `ch.h:181-185` |
| Attributes | `attributes[7]`: `base`, `effective` | `effective = clamp(base + modifier, 1, 125)`. Only `base` is persistent truth | `ch.h:36-41`, `cd.cpp:210-221` |
| Attribute debt | `attribute_point_deficit` | Points still owed toward the profession's minimums (≤ 0) | `ch.h:169-172` |
| Skills | `skills[41]`: `active` (available), `points` (invested), `level` (effective), `base_level`, `practice_count` | See §2.5. `points` is the persistent value; `level` and `base_level` are derived | `ch.h:45-67` |
| Spells | `spell_learned[136]` (one entry per spell id; 1 = learned, 2 = chosen during creation, not yet committed) | The spellbook. Per-realm counts of learned spells are kept in `skill_unlocks[28..33]` and recounted from it | `ch.h:243-251`, `cs.cpp:570-580` |
| Conditions | `uiCondition[20]` (turns left per condition, 9999 = indefinite, 0 = absent); `highest_condition`; `condition_argument` (poison strength) | One duration per condition (table §2.6) | `ch.h:186-204`, `rules/conditions.h:4-28` |
| Enchantments | `enchantments[8]` (`power`, `percent`, `magnitude`, `turns`) | Dracon breath, Guardian Angel, Razor Cloak, Eye for an Eye, Haste, Superman, Body of Stone | `ch.h:17-30`, `rules/conditions.h:40-49` |
| Pools | `uiHPMax`/`hp_current`/`hp_adjustment`; `uiStaminaMax`/`stamina`/`fatigue_penalty`; `sp_max[6]`/`iSPLeft[6]` | HP, stamina, SP per realm. `hp_adjustment` and `fatigue_penalty` are drain booked against the ceilings (cleared when Drained is cured) | `ch.h:205-218`, `src/wiz8/live_party.cpp:181-184` |
| Equipment, backpack | `EquippedItem[12]`, `backpack[8]` | See [inventory](inventory.md) | `ch.h:269-275` |
| Derived (cache) | regeneration rates, weight/capacity/load, initiative, AC, damage reduction, resistances, `Hand[2]` attack blocks, fatigue band, modifier blocks | All rebuilt by §4. Not persistent truth | `ch.h:223-298` |

A rewrite should persist: identity, profession/original profession, race,
gender, portrait, personality, voice, levels, experience, tallies, attribute
bases and debt, skill points (and practice counts), learned spells, condition
durations, enchantments, current pools and drains, and items. Everything else
is derived.

## 2. Enumerations

### 2.1 Races (`rec.h:49-69`; names 644 + id, descriptions 660 + id)

The item `race_mask` is a `u32` and the test is `race_mask & (1 << race)`
(`ir.cpp:606`). Retail masks use 16 bits (`0xffff` all races: 569 items;
`0xffdf` all but Faerie: 167; `0x7fff` all but Android: 18; `0x20` Faerie
only: 11).

| Id | Bit | Name | Playable |
| --- | --- | --- | --- |
| 0 | 0x0001 | Human | yes |
| 1 | 0x0002 | Elf | yes |
| 2 | 0x0004 | Dwarf | yes |
| 3 | 0x0008 | Gnome | yes |
| 4 | 0x0010 | Hobbit | yes |
| 5 | 0x0020 | Faerie | yes |
| 6 | 0x0040 | Lizardman | yes |
| 7 | 0x0080 | Dracon | yes |
| 8 | 0x0100 | Felpurr | yes |
| 9 | 0x0200 | Rawulf | yes |
| 10 | 0x0400 | Mook | yes |
| 11 | 0x0800 | Trynnie | RPC only |
| 12 | 0x1000 | T'Rang | RPC only |
| 13 | 0x2000 | Umpani | RPC only |
| 14 | 0x4000 | Rapax | RPC only |
| 15 | 0x8000 | Android | RPC only |

### 2.2 Professions (`rec.h:26-44`; names 676 + id, descriptions 691 + id, abbreviations 706 + id)

`profession_mask` is a `u16`; the test is `profession_mask & (1 << profession)`
(`ir.cpp:605`). `0x7fff` = all (370 items). The id is also the bit index (Fighter
`0x0001` ... Mage `0x4000`). Table in §3.2.

### 2.3 Gender (`rec.h:74-79`; names 721 Male, 722 Female)

| Id | Item mask bit |
| --- | --- |
| 0 male | 0x1 |
| 1 female | 0x2 |

`gender_mask == 3` admits both. Otherwise the test is `gender_mask & (1 << gender)`
(`ir.cpp:607-609`). Retail: 807 items use 3, 9 use 2 (female only), 2 use 1.
Valkyrie is female only: choosing it forces a male draft to female
(`cg.cpp:818-835`, `cg.cpp:401`).

### 2.4 Attributes (`sk.h:5-14`; names 628 + id, abbreviations 635 + id)

| Id | Name | Id | Name |
| --- | --- | --- | --- |
| 0 | Strength | 4 | Dexterity |
| 1 | Intelligence | 5 | Speed |
| 2 | Piety | 6 | Senses |
| 3 | Vitality | | |

### 2.5 Skills (`sk.h:29-73`; names 738 + id)

The UI groups skills by a per-skill category (`ct.cpp:78-156`) whose labels are
733 Weaponry, 734 Physical, 735 Academia, 736 Magic, 737 Expert. "Governs" is
the attribute pair averaged into `base_level` (`cd.cpp:195-203`). "Who" is
`g_profession_skill_availability` (`ct.cpp:158-200`), by profession abbreviation.

| Id | Skill | Cat | Governs | Who |
| --- | --- | --- | --- | --- |
| 0 | Sword | 0 | STR+DEX | all but MON, PRI, ALC, BIS, PSI, MAG |
| 1 | Axe | 0 | STR+DEX | FIG, LOR, VAL, RAN |
| 2 | Polearm | 0 | STR+DEX | FIG, LOR, VAL, RAN, NIN, MON |
| 3 | Mace & Flail | 0 | STR+DEX | all but SAM, ROG, GAD, BAR, ALC, PSI, MAG |
| 4 | Dagger | 0 | DEX+SPD | all but MON, PRI, BIS |
| 5 | Staff & Wand | 0 | STR+DEX | all |
| 6 | Shield | 0 | STR+DEX | all but NIN, MON, PSI, MAG |
| 7 | Modern Weapon | 0 | DEX+SPD | FIG, LOR, VAL, RAN, ROG, GAD |
| 8 | Bow | 0 | DEX+STR | all but MON, PRI, ALC, BIS, PSI, MAG |
| 9 | Throwing & Sling | 0 | DEX+STR | all |
| 10 | Locks & Traps | 1 | DEX+INT | NIN, ROG, GAD, BAR |
| 11 | Stealth | 1 | DEX+INT | NIN, MON, ROG |
| 12 | Music | 1 | DEX+INT | BAR |
| 13 | Pickpocket | 1 | DEX+SPD | NIN, ROG, BAR |
| 14 | Martial Arts | 0 | DEX+SPD | NIN, MON |
| 15 | Scouting | 1 | SEN+INT | RAN |
| 16 | Close Combat | 2 | SEN+INT | all |
| 17 | Ranged Combat | 2 | SEN+INT | all |
| 18 | Dual Weapons | 2 | DEX+SEN | all but PRI, ALC, BIS, PSI, MAG |
| 19 | Critical Strike | 2 | SEN+SPD | SAM, NIN, MON |
| 20 | Artifacts | 2 | INT+SEN | all |
| 21 | Mythology | 2 | SEN+INT | all |
| 22 | Communication | 2 | INT+SEN | all |
| 23 | Engineering | 2 | INT+DEX | GAD |
| 24 | Wizardry (spellbook) | 3 | INT | SAM, BIS, MAG |
| 25 | Divinity (spellbook) | 3 | PIE | LOR, VAL, PRI, BIS |
| 26 | Alchemy (spellbook) | 3 | DEX+INT | RAN, NIN, ALC, BIS |
| 27 | Psionics (spellbook) | 3 | SEN+INT | MON, BIS, PSI |
| 28-33 | Fire, Water, Air, Earth, Mental, Divine Magic (realms) | 3 | INT+PIE | all but FIG, ROG, GAD, BAR |
| 34-40 | Power Strike, Power Cast, Iron Will, Iron Skin, Reflextion, Snake Speed, Eagle Eye (expert) | 4 | STR, INT, PIE, VIT, DEX, SPD, SEN | anyone, see below |

**Availability** (`csk.cpp:209-279`), recomputed into `skills[i].active` by
`RefreshCharacterSkillAvailability` (`csk.cpp:326-345`):

1. The "Who" table must allow it. The Android's `CANNOT_LEARN` trait also
   removes skills 24-33.
2. A realm skill (28-33) needs an active spellbook skill and at least one
   learned spell in that realm.
3. The profession's four skills and its bonus skill (§3.2) are always available.
4. Expert skill `34 + a` needs attribute `a`'s **base** at 100 or more. Reaching
   100 also announces it in play (`csk.cpp:281-314`).
5. A spellbook skill for a hybrid (magic offset -4) needs
   `profession_levels[profession] - 4 > 0`, i.e. level 5 in that profession.
6. Anything else in the table is available.

**Levels**: `level = clamp(points + bonus(profession) + modifier.skill_bonus[i], 0, 125)`.
The profession's bonus skill adds `max(1, points × 25 / 100)` (`cd.cpp:223-236`).

### 2.6 Conditions (`rules/conditions.h:4-28`)

The name string is 840 + 4·id, except that 16 and 17 are swapped in the string
table (`rules/conditions.h:63-73`). Higher ids dominate: `highest_condition`
≤ 17 (Unconscious) still gets AC terms, and ≥ 18 is out of the party's
ordinary eligibility (`ch.h:193-199`, `cs.cpp:374`).

| Id | Condition | Id | Condition |
| --- | --- | --- | --- |
| 0 | none ("OK") | 10 | Infatuated ("Enthralled") |
| 1 | Drained | 11 | Insane |
| 2 | Diseased | 12 | Blind |
| 3 | Irritated | 13 | Turncoat |
| 4 | Nauseated | 14 | Webbed |
| 5 | Slowed | 15 | Asleep |
| 6 | Afraid | 16 | Paralyzed (908) |
| 7 | Poisoned | 17 | Unconscious (904) |
| 8 | Silenced | 18 | Dead |
| 9 | Hexed | 19 | Missing |

### 2.7 Realms and resistances (`rec.h:114-123`; names 779 + id, abbreviations 785 + id)

0 Fire, 1 Water, 2 Air, 3 Earth, 4 Mental, 5 Divine. The same index selects
the SP pool, the realm skill (28 + id) and the resistance channel
(`ch.h:81-86`).

### 2.8 The item requirement id spaces

The item record keeps **two separate lists** (`include/wiz8/data/item_payloads.h:72-73`,
`src/retail/items.rs`). Neither is tagged; the list decides the space:

| Item field (record offset) | Id space | Test | Retail use |
| --- | --- | --- | --- |
| `attribute_requirements[2]` (0x7d) | attribute 0..6 (§2.4) | `attributes[id].effective >= minimum` | STR (Sledgehammer 50, Strong Bow 85, The Mauler 50), DEX (Zip Gun 30) |
| `skill_requirements[2]` (0x81) | skill 0..40 (§2.5) | `skills[id].level >= minimum` | Engineering 34 items, Music 29, Artifacts 5, Mace & Flail 1, Bow 1 |
| `weapon_skill` (0x46) | skill 0..9 and 14; -1 none | selects the hand's weapon skill (§4.6) | 0..9 in retail |
| bonus pair (0xb1) | skill | added to `modifier.skill_bonus[id]` while worn | |
| bonus pair (0xb3) | attribute | added to `modifier.attribute_adjustments[id]` while worn | |

An id of 0xff is unused (`ir.cpp:578`). Requirements use the **effective**
attribute and the **effective** skill level, so equipment and conditions can
enable or disable other items (`ir.cpp:580-601`). The first unmet one names the
message: attribute name `g_character_description_first_ids[id]` (628 + id), or
skill name 738 + id (`ct.cpp:286-306`). The whole usability rule is in
[inventory](inventory.md) §3.

## 3. Race and profession tables

### 3.1 Races

Base attributes are where a new character starts before any points are spent
(`pri.cpp:30-35`, applied by `cg.cpp:903-907`). Only the 11 playable races have
a row. Resistance adjustments (`ct.cpp:50-67`) add to the resistance base (§4.5).
"Attr/5" means the race's channel gains that attribute's **base** ÷ 5 instead of
a flat amount (adjustment ≥ 1000 encodes attribute `adj − 1000`,
`cs.cpp:718-734`).

| Id | Race | STR | INT | PIE | VIT | DEX | SPD | SEN | Sum | Resistance adjustments | Traits |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | Human | 45 | 45 | 45 | 45 | 45 | 45 | 45 | 315 | - | - |
| 1 | Elf | 35 | 50 | 50 | 35 | 50 | 45 | 40 | 305 | Mental +20, Air +10 | - |
| 2 | Dwarf | 55 | 30 | 50 | 60 | 35 | 35 | 35 | 300 | Fire +VIT/5 | damage reduction |
| 3 | Gnome | 35 | 50 | 40 | 50 | 50 | 35 | 45 | 305 | Mental +VIT/5, Earth +10 | - |
| 4 | Hobbit | 40 | 40 | 30 | 45 | 55 | 50 | 50 | 310 | Earth +VIT/5 | - |
| 5 | Faerie | 25 | 55 | 35 | 30 | 50 | 60 | 45 | 300 | Air, Earth, Mental, Divine +15 | AC +2, capacity ×2/3, three unnamed |
| 6 | Lizardman | 60 | 25 | 25 | 70 | 40 | 50 | 30 | 300 | Water +10, Earth +10, Fire +15, Divine −10, Mental −10 | slow magic recovery |
| 7 | Dracon | 55 | 35 | 30 | 60 | 50 | 40 | 30 | 300 | Water +15, Air +5, Divine −5, Mental −5 | breathe |
| 8 | Felpurr | 40 | 40 | 30 | 35 | 50 | 60 | 50 | 305 | Water −15, Earth +10, Air +10, Mental +10 | - |
| 9 | Rawulf | 40 | 30 | 55 | 50 | 40 | 40 | 50 | 305 | Water +10, Earth +5, Divine +15 | - |
| 10 | Mook | 50 | 50 | 25 | 50 | 35 | 35 | 55 | 300 | Divine +10, Water +15, Mental +15 | - |
| 11 | Trynnie | | | | | | | | | Earth +20, Air +20, Mental −10 | - |
| 12 | T'Rang | | | | | | | | | Mental +15, Water +15 | - |
| 13 | Umpani | | | | | | | | | Earth +15, Air +15 | - |
| 14 | Rapax | | | | | | | | | Fire +45, Mental −20 | - |
| 15 | Android | | | | | | | | | Mental +75, Divine −25 | cannot learn magic, trait 30 (unnamed) |

Race traits: `ct.cpp:27-48`, `sk.h:80-110`.

### 3.2 Professions

Minimum attributes (`pri.cpp:37-43`; "-" = none), HP factor
(`ct.cpp:74-76`), XP weight (`gc.cpp:98-150`), magic level offset
(`ct.cpp:239-241`; -255 = no spellbook), bonus skill (`ct.cpp:202-218`) and
profession skills (`ct.cpp:220-237`).

| Id | Profession | STR | INT | PIE | VIT | DEX | SPD | SEN | HP factor | XP weight | Magic | Bonus skill | Profession skills |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | Fighter | 55 | - | - | 50 | 50 | - | - | 9.0 | 1000 | -255 | Close Combat | Ranged Combat, Sword, Axe, Shield |
| 1 | Lord | 55 | - | 55 | 55 | 50 | 50 | - | 7.5 | 1400 | -4 | Dual Weapons | Close Combat, Sword, Dagger |
| 2 | Valkyrie | 50 | - | 55 | 55 | 50 | 50 | - | 7.5 | 1400 | -4 | Polearm | Close Combat, Mythology, Axe |
| 3 | Ranger | 50 | 50 | - | 50 | 55 | - | 55 | 6.5 | 1400 | -4 | Ranged Combat | Scouting, Bow, Mythology |
| 4 | Samurai | 50 | 55 | - | 50 | 55 | 55 | - | 7.0 | 1400 | -4 | Sword | Close Combat, Dual Weapons, Critical Strike |
| 5 | Ninja | 50 | 50 | - | 50 | 55 | 55 | 50 | 6.5 | 1600 | -4 | Critical Strike | Close Combat, Martial Arts, Throwing & Sling, Stealth |
| 6 | Monk | - | 50 | 50 | - | 50 | 55 | 55 | 6.5 | 1400 | -4 | Martial Arts | Close Combat, Critical Strike, Staff & Wand, Stealth |
| 7 | Rogue | - | - | - | - | 55 | 50 | 50 | 6.0 | 1000 | -255 | Locks & Traps | Dagger, Dual Weapons, Pickpocket, Stealth |
| 8 | Gadgeteer | 45 | 55 | - | - | 60 | - | 55 | 5.5 | 1000 | -255 | Modern Weapon | Ranged Combat, Engineering, Locks & Traps |
| 9 | Bard | 45 | 50 | - | - | 55 | - | 55 | 6.0 | 1000 | -255 | Communication | Music, Mythology, Artifacts |
| 10 | Priest | - | - | 60 | 55 | - | - | - | 5.0 | 1200 | 0 | Divinity | Mace & Flail, Staff & Wand, Communication |
| 11 | Alchemist | - | 55 | - | - | 60 | - | - | 4.5 | 1200 | 0 | Alchemy | Mythology, Throwing & Sling |
| 12 | Bishop | - | 55 | 55 | - | 55 | - | 55 | 4.0 | 1600 | 0 | Artifacts | Alchemy, Wizardry, Divinity, Psionics |
| 13 | Psionic | - | 55 | - | - | - | - | 60 | 3.5 | 1200 | 0 | Psionics | Communication, Mythology, Mental Magic |
| 14 | Mage | - | 60 | - | - | 55 | - | - | 3.0 | 1200 | 0 | Wizardry | Fire, Water, Air, Mental Magic |

Profession traits (`ct.cpp:9-25`): Fighter stamina regeneration, knockout,
berserk; Lord health regeneration; Valkyrie cheat death; Ranger ranged
criticals, auto-search; Samurai fearless, lightning strike; Ninja thrown
criticals, thrown auto-penetrate; Monk damage resistance, effective while
blind; Rogue backstab; Gadgeteer merge gadgets; Bard camp recovery bonus;
Priest pray, turn undead; Alchemist make potions; Bishop remove curses, turn
undead; Psionic fearless, mental-condition immunity; Mage magic resistance
(+5 to every resistance, `cs.cpp:713-715`).

### 3.3 Levels and experience

- **Profession levels count the creation level.** Choosing a profession adds 1
  to `profession_levels[profession]` (`cg.cpp:822-825`), and each level-up adds
  1 to both `uiExpLevel` and the current profession's count (`cg.cpp:152-157`).
  A new character has `uiExpLevel = 1`, `profession_levels[p] = 1`.
- **XP goal** (`gc.cpp:98-150`), with `w` = profession XP weight and the goal
  computed for the level just reached:
  - level 1: `goal = w`;
  - level n in 2..10: `goal = previous_goal + 2^(n−2) × w`;
  - level n > 10: `goal = previous_goal + 256 × w × 1.2^(n−10)` (integer
    `× 12 / 10` per step).

  A Fighter needs 1000 XP for level 2, then 2000, 4000, ... cumulatively.
- **Level band** (`gc.cpp:255-281`): `L = profession_levels[current]`, minus
  `level_band_base` while still in the original profession. Band 0 for L = 0,
  1 for 1, 2 for 2-3, 3 for 4-6, 4 for 7-10, 5 for 11-15, 6 for 16-21, 7 for
  22-27, 8 above 27. The band selects the title string
  (`src/wiz8/local_screens/CharacterScreen.cpp:76-91`; 950 Apprentice, ...).
- **Point pools** (`cg.cpp:113-124`): creation gives 60 attribute points and 15
  skill points. Each level-up gives 6 attribute and 9 skill points.

## 4. Derived stats

### 4.1 Pipeline (`cd.cpp:278-291`, `cs.cpp:828-846`, `ps.cpp:63-117`)

1. Clamp `uiExpLevel` and every profession level to 50.
2. **Equipment block**: fold every worn slot except the alternate weapons
   (8, 9). Each item adds its regeneration bonuses, its skill and attribute
   bonus pairs and its six resistance bytes (the summed resistances are
   clamped to ±125). Its hit and damage bonus are added only from non-hand
   slots (`cd.cpp:140-181`).
3. `bonus = equipment block + condition block + party block` (the party block
   only for members in the party) (`cd.cpp:183-190`).
4. Effective attributes, then every skill's `base_level` (average of its two
   governing attributes' **bases**), then skill levels, then hand kinds.
5. Derive, in order: HP max, stamina max, SP max, regeneration, damage
   reduction, resistances, capacity, weight and load, initiative, attacks, AC.
6. Current pools follow their ceilings: `hp_current += Δmax` (floored at 0;
   reaching 0 kills), `stamina += Δmax`, `iSPLeft[r] += Δmax[r]`. If the fatigue
   band changed, derive once more, because AC reads it (`cs.cpp:499-565`,
   `ps.cpp:65-77`).

"Eq" below means: **D** = reads item fields directly; **A** = depends on
equipment only through effective attributes, skill levels or the bonus block;
**L** = through load.

### 4.2 Pools

| Stat | Formula | Eq | Source |
| --- | --- | --- | --- |
| HP max | `max(1, round(Σ_p (0.6 + 0.008·VIT) × factor[p] × L_p) + hp_adjustment)` over professions with `L_p > 0`, where `L_p = profession_levels[p]`, plus 1 for the original profession | A | `cs.cpp:482-497` |
| Stamina max | `max(0, round((STR + PIE + VIT) / 3 × (0.1·uiExpLevel + 2)) − fatigue_penalty)` | A | `cs.cpp:529-540` |
| SP max per realm r | Skipped (left as is) for Fighter, Rogue, Gadgeteer, Bard. Otherwise `W = min(125, s1 + s2/2 + s3/4 + s4/8)` over the four spellbook skill levels, sorted descending. Then `sp_max[r] = round((W + 3·realm_skill[r] + PIE) × 0.02 × (known[r] + uiExpLevel + 1))`, 0 if no spell is known in r, and at least the highest `spell_point_cost` of a known spell in r | A | `cs.cpp:587-638`, `cs.cpp:653-662` |
| Regeneration per tick | HP `(0.4·HPmax + 20)/240`; stamina `(0.9·STMmax + 20)/240`; SP `(0.65·SPmax[r] + 20)/240` (0 if `SPmax` is 0); each ×1.5 with the block's boost flag | A | `cs.cpp:668-697` |
| Fatigue band | `f = 100 − stamina×100/max`: 0 if f < 50, 1 < 70, 2 < 85, 3 ≤ 94, else 4 | - | `cs.cpp:544-555` |

Attribute names in formulas mean **effective** values unless marked base.

### 4.3 Load, capacity, weight

Already in [inventory](inventory.md) §3: capacity `(VIT + 2·STR) × 12` (Faerie
`× 8`), in tenths of a pound; own weight = equipment + backpack; load band
from (own + pool share) / capacity: none / light / medium / heavy / extreme
(`cs.cpp:778-813`). D (weight) and A (capacity).

### 4.4 Initiative (`cs.cpp:86-115`). Eq: A, L

`(uiExpLevel + 1)/2 + SEN/5 + SPD/5 − 10`, plus `Snake Speed level/10 + 1` if
that skill is active, plus modifier byte 0x00, minus 0/1/2/4/8 by load band.

### 4.5 Resistances (`cs.cpp:699-757`). Eq: A, D (item resistance bytes go through the block)

For each realm r:

1. `base = 25 + realm_skill[28 + r].level / 10`;
2. `+ Iron Will level / 5 + 5` if Iron Will is active; `+5` for a Mage;
3. `+` race adjustments (§3.1);
4. Mental `+ (INT − 80) / 2` if INT > 80; Divine `+ (PIE − 80) / 2` if PIE > 80;
5. `total = min(100, base + bonus.resistance_bonus_all + bonus.resistance_bonus[r])`.
   There is no lower clamp.

### 4.6 Damage reduction (`cs.cpp:759-773`). Eq: A

`Dwarf: VIT/10` + `Monk: trunc(15 × s)` + `Iron Skin active: level/4 + 5` +
block adjustment. `s = (60 + 2·L)/100` for profession level L ≤ 20, else 1
(`cd.cpp:58-74`).

### 4.7 Attacks per hand (`cs.cpp:117-354`, `ir.cpp:128-185`). Eq: D, A, L

Weapon skill = the item's `weapon_skill`, or Martial Arts when the hand is
empty. Combat skill = Close Combat for 0-6 and 14, Ranged Combat for 7-9.
`phys = Σ_p profession_levels[p] × (4 for 0-6, 3 for 7-9, 2 for 10-14)`.
`dual` = both hands hold weapons in play. `dp = dual ? −10(hand+1) − (100 − DualWeapons)/4 : 0`.
Load penalties are halved for Modern Weapon.

| Sheet label | Value |
| --- | --- |
| (combined skill) | `c = (2(combat + 2·weapon)/3 [+ other hand's weapon/2] [+ Dual Weapons]) × 10 / (20 [+5] [+10])` |
| Attack Rating | `(dp + DEX/2 + 2c + phys)/3 + 60` (halved for a damaged Android) |
| # of Attacks | `s = ((SPD+DEX)/2 + dp + phys + load + c)/3`, load 0/−15/−30/−60/−120. Primary hand: 1, 2 if s > 49, 3 if s > 99. Off hand: 1, 2 if s > 74 |
| Max. Swings | `(SPD + 10·dmg + dp + phys + load + c)/3`: 1, 2 if > 66, 3 if > 99, where `dmg` = item damage bonus (unarmed `c/10`). Item kind 0x90 in the primary hand forces 1 |
| To Hit | item hit bonus (+ the partner's for ammunition); STR term (below); DEX `±(DEX−50)/10`; SEN `−(30−SEN)/10` below 30, `+(SEN−70)/10` above 70; load −1/−2/−4/−8 (×1.5 off hand); Eagle Eye (ranged) or Power Strike (melee) `level/20 + 1` |
| To Damage (%) | STR term only (below), plus `10 × dice base` for some generic kinds |
| Damage Range | item dice; unarmed: a 12-row dice table by `Martial Arts level / 11`, +2 base in the primary hand; STR > 49 gives a knockout chance of `(STR−50)/5` |

STR term: weapon kinds 0x68, 0x6e, 0x72, 0x83 and 0x90 ignore Strength, and
some others use divisor `d = 2` (otherwise 1). Below 50: hit
`−(50−STR)/(10d)`, damage% `−(50−STR)/d`. Above 50, with `d ×= hand + 1`:
hit `+(STR−50)/(10d)`, damage% `+(2·STR−100)/d`.

### 4.8 Armour class (`cs.cpp:356-479`). Eq: D, A, L

Components (`include/wiz8/rules/character_stats.h:26-41`), cleared and rebuilt:

| # | Component | Value |
| --- | --- | --- |
| 3 | Shield | Shield items in slots 1, 2, 3, 6, 7: AC bonus, then `+ min(Shield level / 25, shield AC)`. When defending: `/15`, cap ×1.5 |
| 4 | Magic items | Non-shield items in slots 1, 2, 3, 6, 7: AC bonus |
| 0 | Race | Faerie +2 |
| 1 | Speed | +1 above 79, +1 above 89, −1 below 20, −1 below 10 |
| 2 | Stealth | `Stealth level / 10` |
| 11 | Reflextion | `level/20 + 1` if active |
| 5 | Magic spells | block `armor_flat` |
| 8 | Conditions | block `armor_class_adjustment` |
| 10 | Defensive action | +2 while defending |
| 7 | Encumbrance | −1 / −2 / −4 for medium / heavy / extreme load |
| 9 | Fatigue | −0 / 0 / −1 / −2 / −4 by fatigue band (penalty/10) |
| 6 | Vs penetration | block `armor_matchup`, added per location instead of to the total |

Only components 3 and 4 apply above Unconscious (condition > 17). Then:

- `total = Σ components except 6`; at most −5 while out of formation.
- Per location: `total + comp6 + AC of the armour in that slot`. Head = slot 0
  (15%), torso = slot 4 (40%), legs = slot 10 (30%), slot 5 hands (10%), slot
  11 feet (5%).
- **Displayed AC** = `round(Σ weight × location / 100)` (`armor_class_average`;
  CGS/RCS pages and portraits). Combat uses `armor_class_total` and the hit
  location's value (`src/wiz8/combat/attack.cpp:1132-1159`).

## 5. A minimal valid character (test parties)

The NPC initializer is the shortest complete recipe (`npc.cpp:38-92`);
creation adds the point rules (`cg.cpp:889-946`, `cg.cpp:982-1056`).

1. Name; race 0-10; gender 0/1; profession (Valkyrie needs female).
   `original_profession = profession`. Nothing is derived until race,
   profession and gender are all set (`cd.cpp:52-56`).
2. Attribute bases: race base (§3.1), then raise each to the profession
   minimum, then spend the rest of 60 points. One attribute may take at most
   `(60 + 2) / 3 = 20` allocated points and may not pass 100 (`cg.cpp:219-256`).
   A profession is choosable only if its minimums fit within the pool
   (`cg.cpp:398-420`). For a test party, `max(race base, profession minimum)`
   is valid.
3. `uiExpLevel = 1`, `profession_levels[profession] = 1`,
   `experience = 0`, `experience_goal` = XP weight (`gc.cpp:81-93`).
4. Skills: seed the profession skills and the bonus skill with
   `base_level × (60 / (1 + count of profession skills)) / 100` points
   (`cg.cpp:635-673`). The 15 creation points are optional. Then refresh
   availability.
5. Spells: optional (none is valid). A caster may start with chosen spells.
6. `portrait_index` from the sex×race×profession table, or any 0..79;
   `personality` and `voice` from `DeriveCharacterPersonality` (`gc.cpp:319-350`),
   or any 0..8 and 0/1.
7. Derive (§4), then set `hp_current`, `stamina`, `iSPLeft` to their maxima.
8. Starting equipment is optional: a six-item set per profession (Faeries use
   their own set), plus one extra item for Fighter, Priest, Gadgeteer and Bard
   (`cg.cpp:1060-1117`).

A party is six regular members plus up to two RPCs
([party and RPCs](../systems/party-and-rpcs.md)).

Worked check, from the formulas only (not compared with retail): a male Human
Fighter with STR 55, VIT 50, DEX 50 and 45 elsewhere has HP
`round(1.0 × 9 × 2) = 18`, stamina `round(50 × 2.1) = 105`, capacity
`(50 + 110) × 12 = 1920` (192 lb) and initiative `1 + 9 + 9 − 10 = 9`.

## 6. Retail save layout notes

Only where it explains meaning: `record_version` is stamped 1 on save
(`ch.h:131-133`). Records older than version 2 with original profession
Fighter and no Fighter levels have their original profession repaired to the
current one on load (`src/wiz8/save_codec.cpp:833-839`). Attributes are 0x14
bytes (base, effective, a change counter, 8 unknown bytes); resistances 0x10
(`include/wiz8/retail_text_records.h:830-850`).

## 7. Uncertainties

- **Uncertain**: all of this is Whizzardry's reconstruction. The decomp was
  not available.
- **Uncertain**: initiative adds modifier byte 0x00, which Whizzardry names
  `damage_bonus` (`include/wiz8/rules/gameplay_modifiers.h:63`). The field's
  real meaning is unclear.
- **Uncertain**: the AC location table maps location "feet" to slot 5 (hands)
  and "hands" to slot 11 (feet) (`cs.cpp:452-470` vs `character_stats.h:12-19`).
  So feet armour weighs 5% and gloves 10%, or the enum names are swapped.
- **Uncertain**: the spell id base. Realm counting and SP use
  `spell_learned[0..113]` with `spells[i]` (`cs.cpp:575-578`); availability
  reads `spell_learned[i+1]` against `spells[i]` (`csk.cpp:330-333`), and the
  NPC initializer uses 1..114 (`npc.cpp:81-86`).
- **Uncertain**: skill grouping. The category table puts Martial Arts under
  Weaponry and Close Combat ... Engineering together under label 735
  (Academia). The labels may not match the manual's grouping.
- **Uncertain**: the creation pool is a fixed 60 points in Whizzardry. Whether
  retail rolls a bonus pool was not checked.
- **Uncertain**: hybrid spellbooks open at profession level 5 (offset −4,
  counting the creation level). Not checked in play.
- Stamina uses Piety (not Dexterity), and the Dwarf, Gnome and Hobbit race
  resistances use Vitality's base ÷ 5. Both are as coded, but surprising.
- Faerie traits 23, 25, 26 and Android trait 30 are unnamed (`ct.cpp:34-47`).
- The SP formula's per-realm floor uses `spell_point_cost`, which Whizzardry
  names `max_spell_levels` (`cs.cpp:598-606`). Whether it is the base cost per
  power level was not checked.
- The regeneration "tick" length is not established here. See the time domain
  when that system is specified.
