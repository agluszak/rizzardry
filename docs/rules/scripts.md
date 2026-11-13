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
| `CYCLE name` | plays an animation once | its end |
| `DIE` | plays its death | |
| `PLAY volume reach file` | a sound heard from where it stands, `volume` of 127 | |
| `LOOKHERE` | the party turns to look at it | |
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

**Keeping them.** While it is not fighting, is fit to act, and no member of
its group is unfit:

- **Guard.** It walks back to its post when 3 m or more from it.
- **Patrol over places.** It heads for one of them; within 3 m of it, for the
  next, the first after the last. A random patrol takes another of them, each
  as likely.
- **Patrol of an area** (`PATROL near far`). When it has stopped, it makes
  for a point between `near` and `far` metres from where it stands; beyond
  `far` from home, it goes home. (The original walks a path of its own
  graph between the two lengths; only the range is the same.) One that can find no
  way to where it makes for: if it came as a wandering encounter the
  encounter leaves the level; otherwise it tries another point.

- **Look about** (`LOOKABOUT often lasts`). It keeps its orders for
  `often` game minutes, then rests for a spell, then keeps them again, by
  turns; each spell after the first lasts from half to one and a half times
  its number of minutes, at least one. Resting, it stands, and one time in
  twenty that it thinks turns to a heading at random.

Nothing is kept while the party is talking with someone.

## 5. Not built

- `STAYHOME` changes nothing yet. A `DEAF` one hears no noise
  ([monsters.md](monsters.md)).
- How often a resting monster turns is this game's own: the original thinks
  every fifth frame, at no fixed rate.
- `PLAY` and `LOOKHERE` are not waited for; how far a sound carries is not read.

## How this is built

A script is content, read from its file into commands once. A monster with a
script has it as a component with the line it has come to and what it waits
for; one system carries scripts on each step, for monsters not in a fight.
What a command does to the world is done by the event that already exists
for it (a group's stance, a conversation begun, a device run). The named
happenings of `DOACTION` are a table.
