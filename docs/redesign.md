# Redesign: actors, people, and what the story does

A proposal, not yet agreed. It starts from what the code does today, not
from `design.md`. The original is used for formulas and numbers only.

## 0. What is wrong, in one place

| Symptom | Where | Cause |
| --- | --- | --- |
| `AnyOf<((&Character, Option<&Burden>), &Monster)>` | `actors/stats.rs`, `refit` | An actor's sources are two records: a sheet blob, or a species id |
| `content.species[monster.species]` in nine places in `game` | refit, loot, noise, magic | A monster keeps a foreign key, not what its species made of it |
| `impl Character` in five files, 32 `has_trait` calls, `race.row()` and `profession.row()` inside formulas | `characters.rs`, `arms.rs`, `resistance.rs`, `magic.rs` | Formulas hang off the blob and look tables up as they run |
| Earning experience refits the member | `Changed<Character>` | The blob has six lifetimes in one component |
| Refit clamps what is left; drain, disease and lifting a drain move it | `Vitals::refit_maxima` vs `Pool::follow` | Five writers of the maxima, two rules |
| `Vitals` holds the Valkyrie's fight and regeneration fractions | `characters.rs` | Unrelated state parked on the body |
| "A sheet without a seat" filters | time, burden, refit, items | A recruitable's record carries a full character while not being one |
| `NpcBodies(Vec)` + `Deref`, `active_body` scan, six whereabouts components | `npcs/` | A person is a record with things tied back to it |
| 115 consequence variants, nine observers each ignoring most | `npcs/story.rs` | Acts multiplied by how the target is named |

## 1. Layers

Three layers. A component belongs to exactly one, and each layer has one
kind of writer.

| Layer | What it is | Who writes it |
| --- | --- | --- |
| **Sources** | What an actor is made of | Whatever makes the actor; then the one event that changes that source |
| **Body** | What is left of it, what ails it, what is laid on it | Rules that hurt, heal, afflict, lay |
| **Figures** | What rules read to resolve anything | One system per figure, from sources and body. Nothing else, ever |

A rule that resolves something reads figures and body. It never reads a
source, a catalog, a race row or a species.

## 2. Sources

Every type below is a component. "By nature" and "by training" are the two
families of the original's formulas; an actor may have either or both.

### On every actor

```rust
/// The seven attributes before anything is laid on them or ails them.
struct Abilities(EnumMap<Attribute, u32>);

/// Its level: a monster's is its species'; a trained actor's is the sum of
/// its calling's levels, written when it gains one.
struct Level(u32);

/// What it can do that others cannot.
struct Traits(EnumSet<Trait>);
```

### By nature

Written once when the actor is made: from the species for a monster, from
the race for a character.

```rust
/// What it is guarded by, resists, and mends by, untrained and unarmoured.
struct Nature {
    armour: i32,                       // species: 10 - evasion; race: its armour
    armour_at: [i32; 7],               // species only; zero otherwise
    armour_by_mode: EnumMap<AttackMode, i32>,
    hit_locations: Option<[u8; 7]>,
    exposed: i32,                      // what a surprise strips
    fatigue_guards: bool,              // fatigue adds to its guard
    reduction: i32,                    // percent off every blow
    resistance: EnumMap<Realm, Resist>,// flat, or a share of Vitality
    initiative: i32,
    mending: (i32, i32),               // health, stamina a minute
    death_immune: bool,
    carry_factor: u32,
}

/// What it strikes with while holding nothing: a species' attacks. Each is
/// a finished `Arm`, made at spawn. A character has none; its fists are
/// what its hands make of being empty.
struct NaturalArms(Vec<Arm>);

/// The health and stamina it was born with, rolled once.
struct Frame { health: u32, stamina: u32 }

/// Its creature kind, for slaying and holy things and what it cannot suffer.
struct Kind(MonsterKind);
```

Some abilities exist twice today: a character has them by a trait, a
monster by its kind being checked inside refit. Fighting blind is
`Trait::EffectiveWhileBlind` on a character and `kind == Monk` on a
monster; striking from hiding is a Rogue's trait and `kind == Rogue`.
When a monster is made, its kind gives it the same trait, so the figures
ask one question of everyone. Agreed. A trait's strength scales with the
level in the current profession for one who trained, and with its level
otherwise.

### By training

Only on one who trained. This is what replaces `Character`.

```rust
/// What it trained as, and how far.
struct Calling {
    profession: Profession,
    first: Profession,
    levels: EnumMap<Profession, u8>,
}

/// Points put into each skill.
struct Training(EnumMap<Skill, u32>);
```

Not sources (nothing derives from them), so changing them rewrites nothing:

```rust
struct Identity { name, full_name, portrait, personality, voice }
struct Race(..);  struct Gender(..);     // who may use an item; the screens
struct Experience { earned: u64, last_goal: u64 }
struct SpellPicks(..);
struct Practice(EnumMap<Skill, u8>);     // counts towards the next point
struct Tally { kills, deaths }           // exists already
```

### What is taken from it

```rust
/// Health and stamina its maxima have lost until the drain is lifted.
struct Drained { health: u32, stamina: u32 }
```

### What it holds and bears

Unchanged: items by `Worn`/`Packed`, spells laid by relationship,
`Conditions`, `Burden`.

### What goes

`Character`, `impl Character` (all five), `Monster::species` as a key
(`Monster` keeps the placement id and the scaled body), `race.row()` and
`profession.row()` inside any formula, `Attributes::of_sheet`,
`Skills::of_sheet`, `Vitals::new(&Character)`, `Allegiance.npc`-style
copies of species flags that a source now holds.

## 3. The body

```rust
struct Pool { left: u32, most: u32 }

/// Health and stamina, and how health last ran out.
struct Vitals {
    health: Pool,
    stamina: Pool,
    fell: Option<Fall>,     // { from: u32, by: Option<Entity> }
}
```

Moved out, each to where it is used:

- regeneration fractions: a `Recovering { health: f32, stamina: f32 }`
  component, written only by the mending observer;
- the fight a Valkyrie last cheated death in: `CheatedDeath(Entity)`,
  written only by the death rule.

### The maxima: one writer, one rule

`most` is a figure. Only the `maxima` system writes it:

```
health most  = frame.health  + trained health  - drained.health,  at least 1
stamina most = frame.stamina + trained stamina - drained.stamina, at least 0

trained health  = round(sum over professions of
                    (0.6 + 0.008 x Vitality) x profession's factor x levels,
                    the first profession counting one level more)
trained stamina = round((Strength + Piety + Vitality) / 3 x (0.1 x level + 2))
```

A monster has a frame and no calling; a character a calling and no frame.
The same line serves both.

**The rule**, from the original's `RecalculateCharacterHitPoints` and
`RecalculateCharacterStamina`: whenever `most` changes, for any reason,
`left` moves by the same amount, not below nothing. Health at nothing is a
fall, settled like any other.

Everything that pokes the maxima today becomes a change to a source, and
the figure follows:

| Today | Becomes |
| --- | --- |
| `drain_health` lowers `max` and bumps `hp_drained` | adds to `Drained`; tires by two thirds |
| `lift_drain` raises both maxima | clears `Drained` |
| `sicken` lowers a maximum | adds to `Drained` |
| `weaken` after an attribute is lost | nothing: `Abilities` changed, so the figure moves |
| `refit_maxima` clamps | deleted |

**This changes behaviour.** Today a spell or an item that raises Vitality
raises the maximum and leaves what is left where it was. With the
original's rule what is left rises with it, and falls with it when the
spell ends.

Checked in the original: `RecalculateCharacterDerivedStats` rebuilds the
effective attributes and then calls both pool routines, and it is itself
called from every rebuild of a character: when a condition or enchantment
comes or goes (`RebuildConditionsAndDerivedStats`), when a party effect
does (`RebuildPartyEffectBlock`), and when gear changes. Draining goes the
same way (`DamageCharacter` books the loss and calls the health routine).
So a Hex, which takes twenty off every attribute, lowers a member's most
health and stamina and what is left of each by as much, can kill, and
gives it back when lifted. A monster's maxima never move: it has no
Vitality and nothing drains it.

## 4. Figures

The types rules already read stay: `Attributes`, `Skills`, `Resistances`,
`Armour`, `Attacks`, `Mending`. `Vitals::most` joins them.

Each is a sum of parts, one part per kind of source:

```
figure = by nature + by training + worn + laid + ailing
```

A part whose source is absent is nothing. That is the whole of how a
monster and a character differ here: which sources they carry.

### One system per figure

Each is a plain system over exactly the components named. They run in this
order in the settling schedule, each for the actors whose inputs changed.

| System | Reads | Writes |
| --- | --- | --- |
| `attributes` | `Abilities`, `Modifiers`, `Traits` | `Attributes` |
| `skills` | `Option<&Training>`, `Option<&Calling>`, `Modifiers` | `Skills` |
| `maxima` | `Option<&Frame>`, `Option<&Calling>`, `Level`, `Attributes`, `Drained` | `Vitals::most` |
| `resistances` | `Nature`, `Attributes`, `Skills`, `Option<&Calling>`, `Traits`, `Modifiers` | `Resistances` |
| `armour` | `Nature`, `Attributes`, `Skills`, `Traits`, `Burden`, `Modifiers`, what is worn at each place | `Armour` |
| `arms` | `Option<&NaturalArms>`, hands, `Attributes`, `Skills`, `Option<&Calling>`, `Level`, `Traits`, `Burden`, `Modifiers` | `Attacks` |
| `mending` | `Nature`, `Modifiers`, `Traits`, `Option<&Calling>` | `Mending` |

By-training parts take the calling where they scale by profession level
(a trait's strength, a Fighter's wind). One without a calling scales by
its `Level`, which is what the original does for monsters.

### How a figure knows to be rewritten

No stale marker, no noticing systems. Whatever changes an actor's numbers
from outside is a modifier, and the actor carries the running total of
each kind:

```rust
/// What is added to an actor's numbers by what is laid on it, what it
/// wears and what ails it. Every actor has one.
struct Modifiers {
    laid: Bonus,     // the spells on it, and on its party
    worn: Bonus,     // what it wears in the slots in use
    ailing: Bonus,   // its conditions, and being dead
}
```

`Bonus` is the struct a laid spell already carries (armour, to-hit,
resistance by realm, attributes, reduction, missiles, stride), extended
with skills and mending. Each total has one writer, which runs when its
modifiers come or go:

| Total | Written by | When |
| --- | --- | --- |
| `laid` | an observer | a spell is laid on or lifted from the actor; for one on a party, every member's |
| `worn` | an observer | an item is put on or taken off; a system, when a worn item itself changes (out of shots) |
| `ailing` | a system | `Changed<Conditions>`; an observer when `Dead` comes or goes |

Bevy's observers make coming and going symmetric, so nothing needs a
special case for "the last one went". Each writer sums again the few
modifiers of its kind rather than adding and subtracting one, so a spell
replaced by a stronger one cannot leave a remainder.

The figure systems then filter on `Changed` of exactly what they read,
all of it components the actor always has: `Modifiers`, `Abilities`,
`Level`, `Traits`, `Nature`, `Calling`, `Training`, `Drained`, `Burden`,
and the figures before them in the chain (`attributes` changing reruns
`maxima`, `armour` and `arms` in the same settling).

What is in the hands is the one input that is not a number: the `worn`
writer also runs for the weapon slots, so `arms` reruns when a hand
changes.

### What follows a rewrite

The `Refitted` event goes. Its one follower, shedding gear a member can no
longer use, becomes a system on `Changed<Attributes>` or `Changed<Skills>`.

## 5. Making actors

| | Sources written | From |
| --- | --- | --- |
| A monster | `Abilities`, `Level`, `Traits`, `Kind`, `Nature`, `NaturalArms`, `Frame`, plus what it has today (allegiance, worth, temper, special, spellcasting) | Its species, once, in `embody` |
| A character | `Abilities`, `Level`, `Traits`, `Nature`, `Calling`, `Training`, `Identity`, `Race`, `Gender`, `Experience` | Creation, or a recruitable's template |

Events that change a source afterwards:

- gaining a level: `Calling`, `Level`, `Traits`, `Abilities`, `Training`;
- changing profession: `Calling`, `Traits`;
- a skill practised to a point: `Training`;
- disease taking a point: `Abilities`;
- draining and its lifting: `Drained`.

`Traits` is rewritten as race's set joined with the current profession's
whenever the calling changes. It is stored because 32 places read it.

## 6. People

A person has two statlines that are never in play together: their
species' while they stand in the world, their own while they travel with
the party. One actor cannot carry both without a switch inside the
figures. So a person is not an actor; they *are embodied by* one.

### Three kinds of entity, each ordinary

| Entity | Components | Lifetime |
| --- | --- | --- |
| **The person** | `Person(NpcId)`, `Regard` (today's `Npc`), stock, `Alias`, `Whereabouts` | The whole game |
| **Their body** | Everything a monster has | Its level's |
| **Their companion self** (recruitables) | Everything a character has | The whole game; set aside (`Disabled`, with its gear) while unseated |

```rust
/// On the actor a person currently is: their body while they stand
/// somewhere, their companion self while they are seated. One at most.
#[relationship(relationship_target = Embodied)]
struct Embodies(Entity);

/// On a recruitable's companion self, always.
#[relationship(relationship_target = Companion)]
struct CompanionOf(Entity);

/// Where the story has them when they are not in the party.
enum Whereabouts {
    /// Where a level places them.
    AsPlaced,
    /// At a named spot of a level, once the party is next there.
    Posted { level: String, spot: String, greets: bool },
    Gone,
    Killed,
}
```

Nobody reads the person's side of either relationship. Whoever needs the
actor a person is queries for the one that carries `Embodies(person)`.

### What happens

| Event | What changes |
| --- | --- |
| A level places a monster whose species is somebody | The body is made like any monster, with `Embodies`. A placed-only NPC's person is made with it |
| The story posts someone | `Whereabouts::Posted`; a system makes the body when the party is in that level |
| They join | The body's conditions go to the companion self; the body is despawned; the companion self is woken, seated, and takes `Embodies` |
| They leave | What they do not want is returned; the companion self is unseated and set aside; a body is made beside the party like any monster, with their conditions and `Embodies` |
| Their body dies | `Whereabouts::Killed` |
| They die seated | Dead like any member; out of a fight, unseated and set aside |
| A device speaks for them | The device carries `Speaks(person)`; no actor |

### What goes

`ActorId` on members, `NpcBody`, `NpcBodies`, `active_body`,
`NpcLocation`, `Placed`, `Post`, `GreetsOnArrival`, `Gone`, `Killed`,
`NpcTrigger`, `NpcSpeaker`, the `NpcActors` parameter, `Has<NpcBody>` in
combat, and every "sheet without a seat" filter.

### Who is affected

- conversation: its endpoint is the actor carrying `Embodies`, or a device;
- the story's `Joined`, `Killed`, `Stands` readings and remarks by a
  named person: through `Embodies`;
- the party's voice and the two-companion count: `With<Embodies>` on a
  member.

## 7. What the story does

A row names who, and what happens to them.

```rust
enum Row {
    /// These happen to those named.
    To(Who, Vec<Act>),
    /// Something about the story itself.
    Story(StoryAct),      // facts, factions
    Party(PartyAct),      // raise, heal, pick a member, the rest faint
    Level(LevelAct),      // devices, variables, things appearing
    Scene(SceneAct),      // sound, shake, camera, fades
    /// More rows, later or if something holds.
    Flow(Flow),           // If, Later, InHours, AfterTalk, AfterBattle, OneOf
}

enum Who {
    Person(u16),          // by kind
    Leader(u32),          // of the first group of a species
    This,                 // the body whose script or cycle said so
    Chosen,
    Everyone,             // each member fit to
    OfRace(Race),
}

/// One thing that happens to one actor. An entity event.
enum Act {
    Leave, Follow(String), GoTo(String), Show(String), Say(u16), Greet,
    Die, Sleep(u32), FaceParty, Stand(Stance), Remark(u16), Drop(u16, String),
}
```

One resolver turns `Who` into entities. Each `Act` is triggered on each of
them and handled by one observer that works on any actor. `StoryAct`,
`PartyAct`, `LevelAct` and `SceneAct` are each one event type with one
observer that matches exhaustively, so a variant without a handler does
not compile.

`Gone`, `GroupGone`, `BodyGone`, `SelfGone` and `FirstGone` become
`To(who, [Leave])`; likewise the three forms of script, show and move.

`facts.ron` is rewritten in this shape, about 400 rows. The retail test
that checks every fact name against the original's database keeps guarding
it.

## 8. Order of work

Each step builds, passes the unit suite and the retail tests, and is one
commit.

1. **Body.** `Pool { left, most }`; `Vitals` slimmed; `Drained`,
   `Recovering`, `CheatedDeath` out; the one maxima rule. Refit still
   writes `most`, through the one rule.
2. **Character's sources.** `Character` split into `Abilities`, `Calling`,
   `Training`, `Traits`, `Identity`, `Race`, `Gender`, `Experience`,
   `Practice`. `impl Character` formulas become free functions over those.
   The screens and creation follow. Largest step: sixty files.
3. **Monster's sources.** `embody` writes `Abilities`, `Level`, `Traits`,
   `Kind`, `Nature`, `NaturalArms`, `Frame`. Catalog lookups in `game`
   go, except where a thing is made or shown.
4. **Figures.** Seven systems; `refit`, `of_member`, `of_species`,
   `AnyOf`, `Refitted` deleted.
5. **People.**
6. **Story.**

## 9. To decide

1. The three layers and the source list (section 2).
2. The one maxima rule and the behaviour change it brings (section 3).
3. Modifier totals on the actor and change filters on each figure
   (section 4). Agreed in discussion.
4. A person as three ordinary entities (section 6).
5. Rows as who and what, with `facts.ron` rewritten (section 7).
6. The order (section 8).
7. A monster's kind gives it traits at spawn (section 2). Agreed.
