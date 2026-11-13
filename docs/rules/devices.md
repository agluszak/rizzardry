# What the story has to say about a device

This page is the specification for slice 10 of [the plan](../plan.md). Where
the code differs, the code is wrong.

The original keeps a routine per level that names some of its devices and
gives each a callback (`InitializeLevelMasterFunctions` and the files of
`level_specific_code/`). The callback runs when the device is used, after its
action is chosen and before it is carried out, and says whether the device
goes on.

## 1. The rule

A device the story has a say in has **cases**, in order. When it is used, the
first case whose reading holds

- sets off what the case names, and
- says whether the device then does what it was made to.

A device none of whose cases holds simply runs. A device stopped by its case
is silent: the party is not told that nothing happened.

Readings and what is set off are the story's own
([story.md](story.md)): a fact, a variable of the level, what the party
holds; another fact set, a group arriving, a message of the level.

What a case sets off befalls when the device is done, not before it.

A device that speaks for someone asks its cases before they are spoken with;
if it goes on, they are.

**Waiting.** What a case sets off may wait: for so many seconds, for a
named device to stop moving, for a talk to be over, or happen again and
again so long apart while a reading holds. A lift is a chain of the second:
its gate, then the lift, then the other gate, each when the one before has
stopped.

**One of the party taken.** The idol's gas takes one member, never one of
the first two of the party's order: of the rest, one of those wanted least,
at random. A Fighter counts 2, a Rogue 3, a Mage 4, a Priest 5 and anyone
else 1; the least is taken. They are missing until given back.

**Entering a level.** A level may also have cases for the party's entering
it. Every one of them that holds sets off what it names, in order. (The
original runs a setup as a level loads, and much of it only puts back what
loading lost; a level here keeps what was in it, so only what a setup
decides is a case.)

## 2. The cases

They are the table `assets/tables/devices.ron`, by the level's three letters
and the device's name. A name several devices share is the last one's.

## 3. Ported so far

| Level | Devices |
| --- | --- |
| Umpani camp | the three cages |
| Rapax Rift | Al-Sedexus's replacement, the lava lord, the fire ants, Rafe's cage, the ash lock |
| Lower monastery | the roaches, the spiders, the bar, the head in the coffin, the fumes, the wheel |
| Mount Gigas top | the catch cord, the pain plate |
| Arnika to Trynton | the crypt |
| Swamp | the oil pool, the Savant's meeting on the first plane crossed and the gas and fire after it, Crock's return |
| Savant's tower | the bomb's shapes and its blocker |
| Sea caves | the five chests' spirits |
| Rapax castle, main floor | Al-Sedexus's appearing, what she asks for laid out, the platforms |
| Rapax castle, upper floor | the treasury's air box and its door |
| Mount Gigas caves | the mud wall, the pressure plate and its secret door |
| Trynton | the fountain: one of four things, each as likely (one member hexed for 10 to 14 rounds; everyone's spell points filled; everyone healed by 100; everyone given 100 stamina) |
| Trynton's upper branches | the vines and the Zulu, the meat and Hogar, the urns |
| Cosmic Circle | stepping into the arena |
| Outside Mount Gigas | the door, the sentry, the births, the alarm, the crank and the flag |
| Mount Gigas upper caves | the wire panel and its three wires, the officers' doors, the lasers' alarm, two sounds |
| Cosmic Circle, entering | the arena set the first time |
| Upper monastery | the bells (the button rings the bell the dial stands at; the third ends it), what lies behind the small door |
| Marten's Bluff | the hand lock, the three dials and the door they open (7, 4, 0), the gas and its switch, the teleporter pad with its wire and four buttons, the T'Rang arriving every minute, the monitor, Marten's diary |
| Under Marten's Bluff | the arrow trap, the spiked balls, the crusher, the dummy and its lever, the perfume box and the Rapax it brings, the side gate's bolt, the idol and its gas, the blue flowers |
| Arnika | the scanner and its door, the Mook's image and door, the Mook's warning, the pedestal and its three relics, the bank's guards, alarm, exit and golem, both lifts and their buttons, the flight recorder, the ball slot |
| Ascension Peak | entering with two and with three relics; Aletheides's three places; the Savant's arrival; the rumble |

## 4. Known to differ

- The crypt answers "not yet" whenever the party does not hold the item; the
  original also asks that the device's chosen action be its first.
- The Savant, meeting the party in the swamp, is not looked at as he
  appears and goes at once rather than fading.
- The bomb's blocker is not pinned in place as a prop.
- A spell cast where the party stands lands on its living members; the
  original casts it at the place.
- The sea caves' tomb door is not here.
- A device that speaks for someone begins the talk before any case is asked,
  so what the party holds is not offered with it (the lifts and doors of
  Mount Gigas).
- Al-Sedexus's altar does not come to ask for the item it then needs.
- Hogar takes the meat even when asleep.
- The alarms sound once, not for their thirty seconds.
- The original has two switches on a device, "on" and "enabled"; here both
  are the one.
- A wrong wire does not set the three wires' props back.
- Once the shaman has died Trynton's people leave and the dead come in
  their stead. The original also makes a dead one for each who was there,
  at no place its code sets; those are not made.
- On Ascension Peak: which side of the landslide the party is on is not
  decided; the daughter comes whether or not she was met before; the foes
  fighting the party do not die as the Savant's scene begins.
- Stepping into the Cosmic Circle's arena does not end a battle under way.
- A bell rings a moment after its button is pressed, not before the button
  moves.
- Marten's Bluff keeps its door's state as three dials and a switch, not as
  the original's one packed number; the gas hisses once, not until it is
  switched off; a T'Rang arrives even during a battle; the party is not told
  of the lift as it arrives beside it.
- Under Marten's Bluff the dummy is not set out as the level is first
  entered, and taking the idol does not surprise the party.
- In Arnika the vault's alarm turns only the first group of guards; the
  ball reaches the one it is for by way of the party's hand.
- The pressure plate of the Gigas caves is held down only by the party, not
  by things left on it.
- Outside Mount Gigas the flag is not seen to come down its pole, nor to be
  gone once taken; the crank's steps and what they give are there.
- A trap's missile passes monsters by; in the original it strikes whatever
  it meets. Guarding and a braced stance do not count against it.
- The crusher's hiss sounds once. Its door controls shut the open doors
  whichever way the controls stand; the original only when they are off.
- Arnika's scanner does not hold the party still while it sweeps. (The
  original's alarm warning there is only ever restarted from a saved game;
  nothing in the level starts it, so there is nothing to build.)
- Arnika's pedestal: a relic offered while another sits there, before any
  warning, is refused; the original takes it and it is lost. The party is not
  turned to look at the Mook as they warn it.

## 5. What shoots and what crushes

**A trap's launchers** (`FireTrapLauncher`, `ResolveSpellMissileHit`). Each
named launcher fires a missile from where it stands, 30 m along the way it
faces. If the party stands in its way with nothing solid between, it strikes
one living member at random, where a blow lands on a member (head 15, body
40, legs 30, hands 10, feet 5 in a hundred). A Missile Shield turns it aside
at its own chance. Otherwise it gets through armour on a roll of 1 to 100 no
more than `50 + the thrust's 10 + 5 x (10 - the armour there, less what it
has against a thrust)`, and does the trap's dice: 2d6 for both of Marten's
Bluff's. The arrow trap fires once; the spiked balls sixteen volleys two
seconds apart, and their trigger stops them if used while they fly.

**The crusher** (`MartensBluff2MonsterCrusher`). While its two plates close,
a monster in the way of one is pushed along by it; one both reach dies,
except 36 times in 100 when it is pushed clear. Crushing the Rapax (species
0x183) is worth 10,000 experience to each living member. When the plates
have stopped and the raven's quest is under way, the dummy and its rope are
set again.

## How this is built

**A device that speaks for someone** hands them what the party holds as
the talk begins, in place of the greeting (retail
`NotifyNpcTriggerActivation`).

**A device asks.** `triggers::Activating` is an event at the device; whoever
observes it may stop the device. The device's own code knows nothing of the
story.

**The cases are content.** `npcs::devices::Gate` holds a device's cases, given
as its level is entered. No level or device is named in game code.

**One vocabulary.** A case's reading is a `Reading` and what it sets off are
`Consequence`s, carried out by the observers that carry out the story's.
