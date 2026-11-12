# The story so far

This page is the specification for slice 9 of [the plan](../plan.md). It says
what the story's facts are in the game's terms and what follows from them.
The facts, their names and what each sets off are the original's; its
1000-byte array, its 750-line switch and its queue of numbered commands are
not the design.

Sources: `fact_state.cpp` (`EvaluateFact`), `NPC Scripting Facts.cpp`
(`SetFact`, `HandleFactChange`, `InitializeFactState`), `NPC Scripting.cpp`
(`ProcessMessageBoxQueue`), `NPC Manager.cpp` (`MarkNpcOfKind`,
`HandleMarkedNpcEvent`, `UpdateNpcEvents`).

## 1. What a fact is

The story is some eight hundred **facts**, each true or not (a few hold a
small number). A person's script asks facts before it speaks and sets them as
it speaks ([conversation.md](conversation.md) section 3); the journal shows
those that changed, by what the fact's record says.

A new game begins with four set (`FACT_VIRGIN` and the three
`FACT_QUEST_VIRGIN_n`); every other is unset.

## 2. Facts that are read off the world

Most facts are simply what was last set. Some thirty-six are instead read off
the party and the world when asked:

| Kind | Facts |
| --- | --- |
| the party holds an item | Marten's badge, the helm, the IUF pass, the Bloodlust sword; and its opposite (the shaman's helm quest) |
| how many of three devices the party holds | exactly one, two, three; at least two |
| every member holds an item | the wetsuits |
| a faction is friendly | ten factions |
| someone is in the party | Myles, Vi, Sparkle, Rodan, Drazic; two companions; both Rodan and Drazic; a Mook fit to act |
| other facts, combined | either, neither, not both, the same as another |
| a person is gone | Glumph |
| a one-off | the five grenades of the covert training, once seen always so; Vi counted dead while she lies unconscious in the party; the member taken by Al-Sedexus |

## 3. What follows from a fact

Setting a fact may set something off, at once. One hundred and nine facts
do. What they set off is of a few kinds:

| Kind | How many | What it is |
| --- | --- | --- |
| a device runs | 37 | a named trigger of the level is run, sometimes only if it never has; some are switched off; a few set a variable of the level |
| a faction changes | 18 | it becomes friendly when the fact is set, hostile when cleared; one sets a mark on the faction |
| a person's turn comes | 11 | a named person is marked: when the party is next where they are, they are moved, changed or gone, each in their own way |
| a merchant gets an item | 10 | so many of an item, after a day or at once (the smiths' and the alchemists' commissions) |
| a person leaves where they stood | 4 | their body is gone from its level; one is stood somewhere else |
| a group turns | 4 | a named monster group becomes hostile, or friendly |
| another fact | 4 | it is set or cleared too |
| the party changes | 3 | a skill is raised to a floor for those who may have it; Intelligence rises by up to five; a member's absence ends |
| something appears | 2 | items, or a monster, at a named spot |
| a scene | 36 | one of some forty-five scenes: a person walks off along their script, fades, is called or sent home; a door of a pillar opens; the ground shakes; the endgame |

A scene is played after whatever is being said.

## 3a. What follows from a death, and from taking an item

Sources: `NPC Scripting Facts.cpp` (`MonsterKilled`, `HandleScriptedNpcDeath`),
`PC Item.cpp` (`UpdateFactsAfterAcquiringItem`).

The story also hears of two things that are not facts being set. Each is a
table of the same consequences, by species and by item.

**A death.** When one of these species dies, whoever killed it:

| Species | What follows |
| --- | --- |
| Brekek | his death is a fact |
| the Rapax queen | her death is a fact; the Templars' standing rises by 20 |
| a Rattkin breeder | the level counts it; at the second, the Trynnie's quest is done and the common Rattkin turn hostile |
| Al-Sedexus | her quest is cleared if she had attacked; the chosen one is smitten no longer and speaks (event 37); her death is a fact |
| the Rapax king | his death is a fact |
| the Lava Lord | his death is a fact |
| the Dark Savant | Vi, if she is of the party, speaks event 24, and so does one other member when more than two are fit to; everyone fit to speaks event 28; the ending the party chose goes on (the turn to the book, or Phoonzang's greeting); every hostile monster of the level dies |

**An item taken.** When an item comes to a member or to the party:

| Item | What follows |
| --- | --- |
| the Destinae Dominus, the Astral Dominae, the Chaos Moliri | it is possessed, and the quest to get it is done |
| the Chaos Moliri | two days of the game's time after it is first taken, the Mook know it stolen |
| the false Astral Dominae | it is possessed (two facts) |
| three quest items | their quests are done |

An item is the party's (`Owned`) while a member wears or packs it or it is in
the pool or the hand; the story hears of it as it becomes so, not as it is
moved about within the party. The original counts the Chaos Moliri's two days
anew each time it is taken up again; here they run from the first time.

Not here: an NPC's body dying marking the NPC as having appeared.

## 4. The party's own in the story

- **The one Al-Sedexus is offered.** The player picks a member. A woman is
  refused while the party has anyone who is not one. A companion, or a
  member not fit to act, cannot be offered. One of the party's own who
  carries the three things she asks (items 0x1fd to 0x1ff) is chosen;
  otherwise they are "not dressed". The chosen one is smitten with her,
  for good, once the party is out of battle and outside the Rapax castle
  and the Rift, unless the person who spares them stands in the level.
- **Her capture.** Everyone living but the chosen one lies senseless while
  the screen is dark, and wakes as it returns.
- **Missing.** A member taken from the party keeps their place, does
  nothing, and is back only when the story says so.
- **The ending.** The first of the table's endings whose reading holds; the
  party can do nothing more.

## How this is built

**Facts are content.** Which facts begin set, which are read off the world
and how, and what each sets off are a table under `assets/tables/`, keyed by
the fact's own name. No fact is named in game code.

**A fact read off the world is asked of the world.** One accessor answers
"is this fact so" from what was set and from the party, the factions and who
is where. Nothing stores a copy of what it would answer.

**Setting a fact is an event.** What it sets off is carried out by the
observer of each kind of consequence, which the table names. Every fact can be read and set; one the
game's records do not name reads as unset.
