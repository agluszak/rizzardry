# Actors

An actor is anyone who can be hurt and can act: a party member or a monster.
Rules read an actor and never ask which of the two it is.

This page is the specification. Where the code differs, the code is wrong.
Numbers and formulas are the original's and are cited beside the code that
holds them; nothing else of the original's shape is kept.

## What an actor is made of

| Component | The fact it holds | What changes it |
| --- | --- | --- |
| `Attributes` | Strength, Intelligence, Piety, Vitality, Dexterity, Speed, Senses, each 1–125, spells counted | refit |
| `Level` | The level saves, resistance and special attacks weigh | refit |
| `Resistances` | Percent per realm; what it cannot suffer; its creature kind; how strongly it casts | refit |
| `Armour` | Armour overall and by place; what surprise strips; what bracing adds; damage cut; missile deflection | refit |
| `Attacks` | What it strikes with; when it acts; how hard it presses; how it fares blind or from hiding | refit |
| `Vitals` | Health and stamina, each what is left and the most there can be; what ails it, each condition with how long it lasts | being hurt, tired, healed, rested, afflicted, cured, aged; refit moves the maxima |
| `Side` | The side its group or kind has it on toward the party | the events listed under *Side* |
| `Loyalty` | The faction it keeps to, if any | being made |

Conditions live with health and stamina because they are one state: running
out of stamina is a condition, dying lifts them, and what an actor can do is
read from all three together.

A spell laid is not a component of the actor but a thing of its own, laid on
it (see *Spells laid*). The party bears spells too: what is laid on the party
holds for every member.

An actor is made with all of these. A rule that needs one an actor lacks is a
bug in how the actor was made, not a case for the rule to handle.

What is **not** on an actor: where it stands. A monster stands where its body
is; a member stands where the party is. Rules ask for an actor's place and get
it computed from those; nothing copies it onto the actor.

## Refit

The first five components are written when an actor is **refitted**: when
what they are made from has changed. Nothing asks for a refit and nothing
fires one. A few small systems see that a source changed and mark the actor
stale; one system writes the figures of every stale actor anew. The sources:

- the actor is made, or joins or leaves the party;
- the spells laid on it come or go; for a spell on the party, on every
  member;
- a member's sheet changes: a level, an attribute, a skill point;
- a member's load changes;
- what a member wears changes, or a worn thing itself does (out of shots,
  grown into a better one);
- what ails it changes, or it dies or comes back to life.

These run as part of settling bodies (see Fitness), so the figures follow
their sources by the next time rules read them, not inside the operation
that changed the source. Nothing refits on a timer.

Once the components are written, `Refitted` is said of the actor, for what
follows from its figures: a member's seat in the formation, and what they can
no longer wear, which goes to their pack or the pool (retail
`UnequipUnusableItems`).

What refit writes:

- **A monster**: attributes, level, kind, immunities, resistances, armour by
  place and mode, damage reduction, initiative and each attack come from its
  species record. Armour is 10 less the record's value, so lower is always
  better guarded.
- **A member**: attributes from the sheet; level the sum of profession levels;
  resistances from race, realm skill, Iron Will, high Intelligence and Piety,
  and what is worn; immunities from traits; armour from Speed, race, load,
  shield and skill, and what is worn at each place; one thing to strike with
  per weapon slot in play; maximum health and stamina.
- **Spells** on the actor, and on the party for a member, are counted: on
  armour (Armorplate, Bless, Body of Stone, Armormelt), on rating (Bless,
  Enchanted Blade), on resistance (Magic Screen, Element Shield, Soul Shield),
  on missiles (Missile Shield), on attributes (Superman, Haste), on damage cut
  (Body of Stone).

Where the original computes one number differently for a member and a monster
(initiative, armour, to-hit), both formulas are kept. The difference is which
one fills the component, and it lives only in refit.

When refit moves a member's maximum health or stamina, what they have left
stays as it is, held to the new maximum.

## Being hurt

One operation, `hurt`, is how an actor loses health, whatever the cause:

1. Guardian Angel, if laid on it, takes what it can. An angel exactly spent
   stays until a blow gets past it, then lifts. (The angel is a spell laid,
   so whoever hurts an actor asks it first.)
2. Two thirds of what gets through tires it.
3. Health falls by what gets through, to nothing at most.
4. `Damaged` says how much, to whoever shows it.

Stamina run out leaves the actor **exhausted**: unconscious until its stamina
is above 9.

Health run out is death. Death is settled once, for every cause: a Valkyrie may
cheat it (the first time in a fight at half to all her health, otherwise a roll
of what she had), anyone else **dies**. Dying lifts every condition but
infatuation, lifts every spell, empties stamina, and marks the actor `Dead`.
`Dead` being added is the event the rest of the game reacts to: the fight's
kill, the loot, the member's seat. Being raised removes it.

Two other ways health is lost do not go through `hurt`: being **slain** outright
(no angel, no fatigue) and a member being **drained**, which lowers the maximum
until the drain is lifted and takes present health with it.

## Conditions

A condition is one of seventeen, mildest first: Drained, Diseased, Irritated,
Nauseated, Slowed, Afraid, Poisoned, Silenced, Hexed, Infatuated, Insane,
Blind, Turncoat, Webbed, Asleep, Paralyzed, Unconscious. An actor has each at
most once, for a number of rounds or until lifted. Poison carries how hard it
bites.

Conditions are a component of their own beside the body's health and
stamina. Three operations change them:

- **afflict**: a longer duration replaces a shorter, never the reverse. Slowing
  a hasted actor first spends the slowing on the haste; hasting a slowed one
  the reverse.
- **lift**.
- **age**: timed conditions count down and lift at nothing; a poison weakens as
  it runs its course.

What a condition takes off attributes, skills, armour and to-hit
(`assets/tables/conditions.ron`) is written at the refit: an actor's `Skills`
are a component like its `Attributes`, and every rule reads those. Other
rules read a condition where it is needed: the side an actor is on now is its
side turned about while it is Turncoat; its eyes are shut while it is Blind.

## Fitness

No rule asks a body whether it can act. Whoever changes a body changes its
numbers (health, stamina, conditions) and says nothing more. A few small
systems, the `Settle` schedule, run before each frame's rules and after each
moment of a fight, see the change and settle what follows, in this order:

- stamina spent leaves the body out cold; above 9 it comes round;
- health run out fells it (`Fell`, settled as before: a Valkyrie may cheat
  it, anyone else is `Dead`); health back on one of the dead ends that;
- each member's load is weighed (not in a fight);
- an actor whose sources changed is refitted;
- what it is fit for is marked on it.

What an actor is fit for is a marker component, there while it is alive and
nothing ails it as badly as:

| Marker | It can… | Lost at |
| --- | --- | --- |
| `Lucid` | look, keep watch, identify | Insane |
| `CanGuard` | guard a fellow | Blind |
| `CanAct` | act outside a fight (search, trade, rest) | Turncoat |
| `CanFight` | take, or be the target of, a combat action | Asleep |

Rules filter on the markers and react to their coming and going. A round's
end does what it does to bodies, waits for that to be settled, and only then
judges the fight.

## Spells laid

A spell laid is an entity, related to its bearer (an actor, or the party). It
carries which spell it is and the power and boost it was cast at, and, as
components, what is true of it: the turns it has left, what it adds to its
bearer's numbers, what Guardian Angel can still absorb, whether it lasts only
a fight, whether it pulses each round. A bearer has each spell at most once.

- **lay**: replaces the same spell already there; a personal spell (Dracon
  Breath, Guardian Angel, Razor Cloak, Eye for an Eye, Haste, Superman, Body of
  Stone) only if at least as strong.
- **lift**: the entity goes.
- **age**: as time passes, turns count down; a spell at nothing lifts.
- **spend**: Razor Cloak and Eye for an Eye lose a point of power each time
  they answer, and lift at nothing. Guardian Angel loses what it absorbs.

A spell coming onto a bearer or going from it leaves the bearer's figures
stale, and they are written anew as bodies are settled; no operation has to
remember to. Refit sums what the spells on an actor, and on its party, add.

What lasts only a fight (Bless, Armormelt, the shields, the clouds, Ring of
Fire) lifts when the fight ends. The dead keep no spells.

## Side

An actor's side toward the party is stored on it, and written only when one
of these happens:

- it is made, or comes into a group: it takes the side its group stands on,
  or else what its kind and faction make it;
- its group is made to **stand** some way (the party provokes it, comes within
  its hostility radius, or its faction's fighting leader is seen), or that
  wears off: as time passes outside a fight, once the leader's grudge
  (Intelligence squared, in game minutes) has run out or its faction's band
  has moved since;
- the party's standing with its faction changes, for those nothing has set a
  stance on;
- it joins the party, which makes it friendly.

Turncoat is not written into the side. It is a condition, and the side an
actor is on *now* is its stored side turned about while Turncoat holds: hostile
for friendly, friendly for hostile, a neutral unmoved. One under Turncoat has
also broken with its faction.

Dying fixes the side an actor was on as it died, since death lifts what had
turned it; that is what its kill is credited by. A member raised is the
party's again.

## Place

A rule that needs where an actor is asks for its **place**: feet, eye, centre,
radius, half height and facing. A monster's is its body's position and its
size. A member's is the party's. It is computed when asked; nothing stores it.

What an actor's eyes are worth (its Senses and what its kind adds; shut when
it cannot fight, or is blind and does not fight blind) and how hard someone is
to make out from behind (a monster by its level; the party by its lowest able
level and Chameleon) are likewise computed when asked, from the facts above.

## What is read as it stands

- **fatigue**, from stamina spent: it takes from aim and from pressing past a
  guard, and adds to a monster's armour;
- **bracing**, from defending this round;
- **being on the move**, from walking or the party running this round;
- **place**.

## Condition modifiers

Source: `Gameplay Mods.cpp`, `ApplyConditionModifiers`; `character_skills.cpp`,
`ResetCharacterAttributes` and `GetEffectiveSkillLevel`; `GameplayCode.cpp`,
`CalcArmorClasses`.

| Condition | Attributes | To hit | Armour | Skills |
| --- | --- | --- | --- | --- |
| Irritated | — | −2 | −2 | — |
| Nauseated | — | −5 | −4 | — |
| Slowed | Speed −50 | — | — | — |
| Afraid | — | −3 | −2 | — |
| Poisoned | — | −2 | −2 | — |
| Hexed | All −20 | −5 | — | All −20 |
| Insane | Intelligence −50 | — | — | — |
| Blind | Senses −50 | — | — | — |
| Webbed | Dexterity −50 | — | — | — |

Modifiers add together, including spells, before attributes are clamped to
1–125 and skills to 0–125. The profession's skill bonus is added before that
clamp. A member with Effective While Blind offsets the Senses penalty by
`scale_by_level(profession level, 50)` and keeps their formation place.
Monsters receive the ordinary blindness attribute penalty, even when their
kind allows blind combat.

Blind members without that trait, and Webbed, Asleep, Paralyzed or Unconscious
members, give up their formation place while affected, as the dead and the
missing do. They return to their
remembered row if it has room. Their global armour is capped at −5 before
local equipment armour is added. A member worse than Unconscious (dead or
missing) loses race, speed, skill, spell, condition, load and defending armour
contributions; worn armour remains.

Stealth contributes its effective level divided by 10 to armour. An active
Reflextion skill contributes its effective level divided by 20, plus one.
