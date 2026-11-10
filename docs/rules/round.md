# A round of a fight

This page is the specification for slice 6 of [the plan](../plan.md). It says
what a round is in the game's terms. The numbers are the original's and are
cited beside the code that holds them; its phase counter and its routines are
not the design.

## What the player sees

A fight is played in rounds. Between rounds the player gives each member an
order and may choose how the party moves. Then the round is **played**: one
at a time, whoever's moment has come does one thing, the player sees it and
what came of it, and the next moment follows. When nobody has anything left
to do, the round **ends**, time passes for everyone, and the player orders
again or the fight is over.

## The round's timeline

A round runs from moment 1 to moment 100. Everything that will happen in it
has a moment:

- each actor's **first act**: the earlier the more initiative they have, with
  a roll (members' and monsters' initiative come from refit; the moment from
  the original's roll on it);
- the **party's move**, if the player chose one, at its own moment; members do
  not act before it;
- each **further attack** of an actor who has several: set when the previous
  one is made, spread over what is left of the round, later if they change
  hands to a slower weapon.

The round always plays the earliest moment still to come. Two at the same
moment go in the fight's order: as they joined it. The bar the player sees is
how far along the timeline the round is.

## An order is accepted when the round begins

What a member will do is fixed as the round begins: the order they had then,
and the weapons they held then. Changing an order during the round changes
the next round. An attack is locked to the weapon it was accepted with: one
who no longer holds it loses that attack.

## An actor's moment

When an actor's moment comes, one thing happens, as one piece:

1. If they can no longer fight, or left the fight, they lose it.
2. A member does what they were ordered; anyone else decides now, from what
   they see now.
3. What ails them may take it from them: the insane may turn on their own,
   the afraid may run.
4. They do it: attack, cast, breathe, use, move, flee, or hold.
5. What it did lands, and deaths are settled, before the next moment.

Defending is not a moment: it lasts the round.

## Between moments

The next moment waits for what the player is still watching: a missile in the
air, someone walking their move, and a short pause after each act. These are
part of the game's pace, the same with or without a screen.

## The end of a round

Once the timeline is empty and nothing is in the air:

1. a round of game time passes for everyone (conditions, poison, recovery);
2. groups with nothing left to fight for leave;
3. those who held back get their breath;
4. what burns or hangs over someone pulses on them, and what that does lands;
5. spells laid age;
6. the fight is over if no hostile stays in it, or nobody has attacked or
   closed in for two rounds; the living share what its kills were worth;
7. the party looks again at who it fights.

## What is stored, and where

- On the fight: whether a round is being played and how far along its
  timeline it is; whether anyone has attacked in it; the party's move.
- On an actor in the fight, for the round only: its next moment; the attacks
  it has left with each hand and the weapons they were accepted with; what the
  round did to it (it moved, it held back, it made its special attack, it
  turned on its friends).
- On a member, for the round only: the order accepted.

Leaving the fight, or the round ending, removes what is for the round only.

## What goes

The step counter that visits every moment from 1 to 100 looking for someone
due; the chain of messages that cuts one actor's moment into `Called`,
`Chosen`, `Perform` and separate systems for each kind of act; `RoundPhase`;
turn fields that mirror the original's (`current_hand`, `hand_attack_values`)
under other names where a simpler fact says the same.
