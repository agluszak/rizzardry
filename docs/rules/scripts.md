# What a monster is told to do

A monster placed in a level may have a **script**: a short list of things to
do, in order, while it is not fighting. The story gives scripts too
([story.md](story.md)). This is how a person turns to the party and speaks
when it comes near, how a guard warns and then attacks, and how someone walks
off when the story sends them.

Sources: `Monster.cpp` (`ProcessScript`, `EvaluateScriptCondition`,
`CanContinueScript`, `ResolveScriptPosition`), the 49 scripts under
`data/monsters/scripts`.

## 1. A script

A script is lines, one command each, read top to bottom. A line ending in a
colon is a **label**. A script ends when it runs off its last line or says
`END`; the monster then has no script.

Up to fifty commands are carried out in one step; a `GOTO` ends the step, so
a script that loops looks again each step.

## 2. Where and whom

A place is `PARTY`, `HOME` (where the monster was placed), or a named spot of
the level.

`IF c` … `ELSE` … `ENDIF` skips lines by a condition, `IF NOT c` by its
opposite. The conditions are:

- `PARTYNEAR(n)`: the party is within n metres and the monster has a clear
  line to it;
- `RANDOM(n)`: n times in a hundred.

## 3. What it can do

| Command | What happens | It waits for |
| --- | --- | --- |
| `GOTO label` | carries on from the label | the next step |
| `WALKTO place` | walks there | arriving |
| `FACE place` | turns toward it | |
| `TELEPORT place` | is there | |
| `SAY n` | says quote n of its own lines, heard as it stands | the line |
| `NPCINTERACTION [n]` | begins talking with the party, with quote n if given | the conversation ending |
| `DISPOSITION d` | its group becomes neutral, hostile or friendly | |
| `TRIGGER name` | a device of the level runs | |
| `DELAY ms` | nothing | that long |
| `CYCLE name`, `PLAY …`, `LOOKHERE`, `DIE` | an animation, a sound, the camera, a death played out | |
| `GIVE item` | the party gets an item | |
| `DISAPPEAR`, `FADEOUT` | it is gone from the level | |
| `DOACTION name` | one of a few named happenings of the story | |
| `END` | the script is over | |

A command followed by `NOBLOCK` does not wait.

## 4. Standing orders

Between `BEGINORDERS` and `ENDORDERS` a script says how the monster keeps
itself from then on: `GUARD place`, `PATROL distance variation`,
`POINTPATROL` or `RANDOMPOINTPATROL` over places, `FACE direction` or
`FACE PARTY`, `TURNTOFACEPARTY`, `DEAF` (it does not hear), `STAYHOME`,
`LOOKABOUT`. Reaching `ENDORDERS` ends the script for good.

## 5. Not built

- A monster with standing orders faces as told, but does not yet guard,
  patrol or look about: outside a fight monsters do not move of their own
  accord ([monsters.md](monsters.md)).
- `CYCLE`, `PLAY`, `LOOKHERE` and `DIE` are told to the screen and not
  waited for.

## How this is built

A script is content, read from its file into commands once. A monster with a
script has it as a component with the line it has come to and what it waits
for; one system carries scripts on each step, for monsters not in a fight.
What a command does to the world is done by the event that already exists
for it (a group's stance, a conversation begun, a device run). The named
happenings of `DOACTION` are a table.
