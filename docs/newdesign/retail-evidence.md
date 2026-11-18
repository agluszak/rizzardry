# Retail evidence for story and inventory design

All paths below are relative to the sibling wizardry-8-decomp checkout.
The comparison follows the purchase and acquisition callers into the fact
handler. It establishes source behavior; retail was not run interactively.

## Acquisition follows storage

| Path inspected | Observable rule |
| --- | --- |
| src/wiz8/local_code/PC Item.cpp, AddItemToParty, lines 3322–3362 | Merge in pool order. A full merge or successful new row reaches the acquisition handler even when announcements are off. With a full pool, a partial merge remains applied and the remainder returns to the caller without an acquisition report from this attempt. |
| Same file, AddItemToCharacter, lines 1470–1545 | Successful direct equipment placement returns early. Automatic storage that finishes through the backpack path calls acquisition and then exceptional-item reaction. There is an indexing quirk: an automatic equipped-stack merge reports the backpack cell at the stored index. Its practical effect on quest items is unverified. |
| Same file, MergeItems, lines 1368–1426 | A recipe writes its product directly or inserts a pool row. Neither route invokes the acquisition handler. A product appearing in the party's possessions is insufficient evidence of a quest event. |
| src/wiz8/local_code/NPC Manager.cpp, ReturnDismissedNpcItems, lines 412–439 | Returned gear goes through AddItemToPartyOrDrop. Successful storage can report acquisition of something the party already carried. |
| src/wiz8/local_code/CharGeneration.cpp, calls at lines 1221 and 1250 | Starting gear requests equipment placement. The branch actually taken determines acquisition. Character selection suppresses the separate exceptional-item reaction. |
| src/wiz8/local_screens/MainGameScreen.cpp, buy branch at line 7645; src/wiz8/npc_items.cpp, CompleteNpcItemPurchase, lines 781–840 | The purchase offers each quantity to the pool with drop fallback. It counts attempted purchase units even when the pool cannot store them, consumes stock and charges for the quantity. Preserve charge counts on purchased charged items. |

**Design consequence.** A delivery result identifies units stored, remainder
and its disposition. The successful automatic-storage branch invokes the quest
rule once for that report. Presentation can separately announce storage or
overflow. Ordinary placement, item creation, and roster membership changes do
not synthesize acquisition. For non-discardable overflow, use the held-item
drop-or-refuse behavior; avoid promising that every overflow item reaches the
ground. The decomp's edge case reporting the wrong backpack cell requires a
player-visible example before choosing compatibility behavior.

## Immediate and delayed story effects

In src/wiz8/local_code/NPC Scripting Facts.cpp, SetFact (1026–1051) stores
before dispatch. Suppression skips journal and handler; equality skips only
journal recording. GetFact evaluates on demand (1003–1024). In PC Item.cpp,
UpdateFactsAfterAcquiringItem (3054–3101) reads Astral possession, sets it if
needed, then reads the active quest. The possession handler in NPC Scripting
Facts.cpp (152–161) clears the scheduled Astral Dominae on NPC kind 0x48
before control returns. Preserve the order even though this example does not
show the NPC change altering the subsequent quest predicate.

For the Chaos Moliri, possession and quest writes are conditional, but every
reported acquisition resets one pending theft timestamp. NPC Manager.cpp
(2212–2216) clears the pending flag and sets the theft fact when elapsed
world-clock seconds exceed 172800. Model one campaign-owned optional timestamp.
A second report replaces it. Compare in whole game seconds and fire on the first
eligible update after the strict threshold. Two independent delayed entities
would let the older deadline announce the theft too early.

Item IDs 0x201 and 0x27c unconditionally clear their quest facts. Item 0x1d2
checks first. Preserve that distinction because same-value writes can run
handlers. The exceptional reaction for the diamond runs after acquisition and
itself reads quest state (PC Item.cpp, DeliverExceptionalItemReaction).

## Outcomes to drive through gameplay

- Buy an ordinary discardable item with a full pool. The overflow follows
  the drop path; the merchant loses the purchased stock and the party pays.
- Return a companion's quest item to the pool. Successful automatic storage
  reaches the quest rule even though the item never left party possession.
- Report the Chaos Moliri twice a day apart. The first deadline has no effect.
  At exactly two days after the second report the theft is still pending.
- Allocate this level's free magic skill points at a spell eligibility boundary.
  CountRemainingSpellPoints in src/wiz8/local_code/CharGeneration.cpp (640–713)
  subtracts those points from both raw and effective book and realm skills for
  the learning check. It refunds invalid selections and caps the pick pool by
  available spells. Preview and acceptance must agree on that basis.

## Monster groups and targeting

Paths below are under src/wiz8 in the decomp checkout.

| Source | Behavior that constrains the ECS design |
| --- | --- |
| level_specific_code/SeaCaves.cpp, lines 89–107; local_code/MonsterGroup.cpp, GiveBirthToMonster, line 1601 | An NPC already has an addressable body in the birth cycle. The script finds its group through that body. Birth moves the group and its existing members into play, refreshes sight, and returns failure on a repeat. |
| level_specific_code/Rift1.cpp, Rift1TimeDorado, line 113 | Birth can select all existing groups of a species. The underlying birth routine activates existing members. The caller comment describing creation of fresh members disagrees with its callee. |
| local_code/MonsterManager.cpp, line 212; local_code/MonsterGroup.cpp, ElectGroupLeaderMember, line 1245 | Rank is species effective level times 65536 plus location ID. Election takes the highest positive eligible rank, falling back to the first remaining member. A successor inherits the named script and the previous leader's heard-noise position and radius. |
| local_code/Targeting.cpp, PickNearestMonsterUnderCursor, line 834; CompareMonsterTargetCandidates, line 941 | Cursor distance comparisons are strict, so equal distances preserve scan order. Automatic selection ranks candidate state, reach and distance. A global entity-ID or placement-number tie breaker is unsupported by this comparator. |
| local_code/MonsterAI.cpp, line 239; local_code/MonsterGroup.cpp, RefreshMonsterGroupHostility, line 268 | Active independent groups refresh outside their own combat. At exactly species Intelligence squared minutes a stamped grudge still holds unless its faction timestamp is newer. A strictly later refresh can restore the default. Finite radius hostility checks distance and visibility. |
| local_code/Combat Hostility.cpp, SetMonsterGroupHostility and SetMonsterHostility, lines 380–451 | An all-dying group, a protected leader, or an unchanged stance short-circuits the group operation. A changed stance propagates to the encounter leader and the first two allied slots. An individual protected body can keep its own disposition. Wider faction alerting depends on the request. |

Pairwise disposition also depends on the participants. MonsterHostility (line
68) first treats species 0x224 as neutral, then equal dispositions as friendly
(including two neutrals). With differing non-neutral dispositions, matching
nonzero factions give neutrality only when the Turncoat flags agree. Opposing
bands with differing flags are hostile. MonsterVsCharDisposition (line 107)
uses the monster's disposition and reverses hostile/friendly for a turned
character. CharacterVsCharacterDisposition (line 132) gives friendship for
matching Turncoat status. A shared rule must preserve these distinctions.

Drive the design through a named NPC queried before birth, two equally leveled
successors with different location IDs, and a protected member of a group whose
stance changes. Check a grudge at the threshold and at the next eligible refresh.
The inspected targeting routines establish cursor and automatic attack behavior;
spell victim ordering and death/resurrection disposition remain separate audits.
