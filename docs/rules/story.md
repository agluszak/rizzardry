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

## 4. What this needs that the game does not have

Each is a prerequisite of some kind above and is built before it.

- **Named triggers run from outside their level's script**, and switched off.
- **A faction set to a band** rather than nudged.
- **A merchant's stock added to later.**
- **A person's presence in the world changed by the story**: marked, removed,
  stood elsewhere, and what each marked person then does.
- **Monster scripts** (`.msf`): the paths and poses a body is put through.
  Most scenes are one of these on a named group.
- **The endgame.**

## How this is built

**Facts are content.** Which facts begin set, which are read off the world
and how, and what each sets off are a table under `assets/tables/`, keyed by
the fact's own name. No fact is named in game code.

**A fact read off the world is asked of the world.** One accessor answers
"is this fact so" from what was set and from the party, the factions and who
is where. Nothing stores a copy of what it would answer.

**Setting a fact is an event.** What it sets off is carried out by the
observer of each kind of consequence, which the table names. A fact whose
consequences this game cannot yet carry out is refused before it is set, and
the conversation that needs it says so; when every kind is built, that
refusal and the conversation's check for it are deleted.
