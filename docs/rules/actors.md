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
| `Vitals` | Health and stamina, each what is left and the most there can be | being hurt, tired, healed, rested; refit moves the maxima |
| `Conditions` | What ails it, each with how long it lasts | being afflicted, cured, aged |
| `Spells` | The spells laid on it, each with its power and how long it lasts | a spell being laid, lifted, aged, spent |
| `Side` | Friendly, neutral or hostile to the party | the events listed under *Side* |

The party is a bearer of `Spells` too: what is laid on the party holds for
every member.

An actor is made with all of these. A rule that needs one an actor lacks is a
bug in how the actor was made, not a case for the rule to handle.

What is **not** on an actor: where it stands. A monster stands where its body
is; a member stands where the party is. Rules ask for an actor's place and get
it computed from those; nothing copies it onto the actor.

## Refit

The first five components are written when an actor is made and when it is
**refitted**. `Refit` is an event fired at one actor. Exactly these fire it:

- the actor is made, or joins or leaves the party;
- a spell is laid on it or lifted from it — for a spell on the party, at every
  member;
- a member's sheet changes: a level, an attribute, a skill point;
- a member's load band changes;
- a member's worn gear changes.

Each is fired by the operation that made the change, at the moment it makes
it. Nothing polls for changes and nothing refits on a timer. (Worn gear is the
one exception until items are things: it is noticed by a write to what the
member carries. Slice 3 removes it.)

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
   stays until a blow gets past it, then lifts.
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

What an actor can do follows from its worst condition:

| It can… | when nothing is as bad as |
| --- | --- |
| act outside a fight (search, trade, rest) | Turncoat |
| look, keep watch, identify | Insane |
| guard a fellow | Blind |
| take, or be the target of, a combat action | Asleep |

Three operations change conditions, and each says what it did:

- **afflict**: a longer duration replaces a shorter, never the reverse. Slowing
  a hasted actor first spends the slowing on the haste; hasting a slowed one
  the reverse.
- **lift**.
- **age**: timed conditions count down and lift at nothing; a poison weakens as
  it runs its course.

`Afflicted` and `Recovered` are events fired at the actor when a condition
begins and ends. Whoever cares reacts: its side when it is Turncoat, its eyes
when it is Blind or cannot fight, the screen.

## Spells laid

A spell laid is a spell id, the power and boost it was cast at, the turns it
lasts, and for Guardian Angel what it can still absorb. A bearer has each spell
at most once.

- **lay**: replaces the same spell already there; a personal spell (Dracon
  Breath, Guardian Angel, Razor Cloak, Eye for an Eye, Haste, Superman, Body of
  Stone) only if at least as strong. Laying refits the bearer.
- **lift**: refits the bearer.
- **age**: turns count down; a spell at nothing lifts.
- **spend**: Razor Cloak and Eye for an Eye lose a point of power each time
  they answer, and lift at nothing. Guardian Angel loses what it absorbs.
  Spending does not refit: neither changes a number refit writes.

What lasts only a fight (Bless, Armormelt, the shields, the clouds, Ring of
Fire) lifts when the fight ends.

## Side

An actor's side toward the party is stored, and changes only when one of these
happens:

- it is made;
- its group's stance is set, or a stance set for a while runs out;
- the party's standing with its faction changes;
- it joins or leaves a fight;
- it becomes or stops being Turncoat, which swaps hostile and friendly.

A member is friendly unless Turncoat. A dead actor keeps the side it died on,
which is what its kill is credited by.

## Place

A rule that needs where an actor is asks for its **place**: feet, eye, centre,
radius, half height and facing. A monster's is its body's position and its
size. A member's is the party's. It is computed when asked.

Whether a monster's eyes are open (it can fight and is not blind, unless it
fights blind) and how hard an actor is to make out from behind (a monster by
its level; a member by the party's lowest able level and Chameleon) are
likewise computed when asked, from the facts above.

## What is read as it stands

- **fatigue**, from stamina spent: it takes from aim and from pressing past a
  guard, and adds to a monster's armour;
- **bracing**, from defending this round;
- **being on the move**, from walking or the party running this round;
- **place**.
