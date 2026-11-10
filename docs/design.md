# Design rules

Rizzardry is Wizardry 8's game, rebuilt as a Bevy game. It is not Wizardry 8's
program translated into Rust. These rules say what that means in practice. They
replace the earlier agent guidelines; where code disagrees with them, the code is
what is wrong.

## 1. What the original is evidence of

The decomp (`../wizardry-8-decomp`) and Whizzardry (`../whizzardry-8`) are the
source for three things and nothing else:

- **formulas** — how a number is computed from other numbers;
- **constants and tables** — the numbers themselves;
- **observable behaviour** — what the player can see happen, and in what order
  when the order is visible.

They are not the source for procedures, record layouts, managers, schedulers,
indices, or the order in which unobservable work is done. A retail routine is
never the unit of design here, and no function exists because a retail function
does.

Design starts from two questions: *what is this thing in the game*, and *what
happens to it*. Only then is the original consulted, for the numbers.

Corollaries:

- Do not invent a rule the original answers. Look it up; cite it where the
  number lives.
- Do not reproduce the original's random-number order, intermediate state, or
  quirks that the player cannot observe.
- A comment names a retail routine only to say where a formula or constant came
  from, beside that formula or constant.

## 2. What things are

- **An actor is one kind of thing.** A party member and a monster carry the same
  components and are read by the same rules. What differs between them is where
  their numbers come from, and that difference lives where the actor is made or
  changed, never in a rule that reads it.
- **A thing that exists apart from one owner, or that others refer to, is an
  entity.** An item, a spell in the air, a fight. Identity is the entity, not a
  counter we allocate. What is wholly a state of one owner, at most one of each
  kind, is that owner's component: what ails an actor, the spells laid on it.
- **A fact has one authority.** If a value can be computed from others, either
  it is computed where it is read, or it is stored and changed by exactly the
  events that change it. It is never stored *and* recomputed on a timer.
- **Content is read where things are made.** A catalog record becomes components
  when its thing is spawned. Rules read components; they do not hold an index and
  look the record up again.
- **What persists, persists.** A level that is left keeps its entities. Nothing
  is copied into a record in order to be rebuilt from it later.

## 3. How things happen

- **Things change because something happened**, and what happened is an event.
  A stat changes when gear, a level or a spell changes it. Nothing is re-derived
  every frame in case it changed.
- **A rule is a function of the things it is about.** It takes them, reads and
  changes them, and says what happened. It does not know which screen is open or
  how it was scheduled.
- **Domains talk by events.** Combat does not reach into loot, loot into
  factions, factions into dialogue. One says what happened; whoever cares reacts.
- **A sequence the player sees as one moment is one piece of code.** Do not cut
  an indivisible action into systems joined by marker components.
- **Data is data.** A table is a table in content, not a `match` in a function.
  Logic that is the same for every row is written once.

## 4. Taste

- This is a game, not a service. No defensive checks for states the game cannot
  reach, no transaction or revision machinery, no error taxonomies.
- Plain arithmetic. Clamp where the rule clamps and say so.
- Prefer deleting to wrapping. A layer that only forwards is a layer to remove.
- No abstraction for one user. No framework for a case that does not exist.
- Tests assert rules and what the player would see. A test that pins a layout,
  a schedule or an internal step is deleted, not maintained.

## 5. Boundaries

`retail` decodes the original's files into `content`. `content` is plain data.
`game` is the rules and the state, and runs without a window. `presentation`
shows game state and turns input into requests; it owns no rule. `app` composes
them. Nothing imports upward.

## 6. Recording a rule

When a rule of the game is worked out, it is written in `docs/rules/`, in the
game's terms, with where its numbers come from. The code implements that page.
The page is the specification; the previous implementation is not.
