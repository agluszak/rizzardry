# Talking with someone

This page is the specification for slice 8 of [the plan](../plan.md). It says
what a conversation is in the game's terms. The numbers and the script are the
original's and are cited beside the code that holds them; its message queue,
its modal dialogs and its screen layouts are not the design.

Sources: `NPC Scripting.cpp` (what a quote says and does), `NPC Manager.cpp`
(standing, the trade pool, charm and small talk), `NPCInteractionSubscreen.cpp`
(what the party may do and how the person answers).

## 1. What the player sees

The party walks up to someone and talks. The person **says** things, one line
at a time, each with its voice. Between lines the party may **do** something:
ask about a topic, answer a question, pay or refuse a price, offer an item or
gold, charm or chat, ask the person to join, trade, or leave. What the person
says next follows from what they are (their script), how they stand toward the
party, and the story so far.

While it lasts the party does nothing else: it does not move, fight, camp or
work on a device. There is at most one conversation.

## 2. What a person has to say

A person's script is a list of **quotes**, numbered. A quote is one or more
lines spoken in order, and a list of **entries**: what the quote does once its
last line is reached. The low numbers mean the same for everyone:

| Quote | Said when |
| --- | --- |
| 0 | the first greeting |
| 1 | a later greeting, by a friend |
| 2 | a greeting by one who is neither friend nor foe |
| 3, 4, 6 | small talk heard coldly; a charm that won them; a charm heard coldly |
| 7 | a gift or gold was not enough, or not wanted |
| 0xb, 0xc, 0xd | will not join; no room in the party; joins |
| 0x10, 0x11 | thanks for a gift; will not take that item |
| 0x12, 0x14 | names a price; the party cannot pay it |
| 0x18 | wants nothing to do with the party |
| 0x1c, 0x1d, 0x1e | does not know (one of two); may not say |
| 0x20, 0x21 | too many things asked at once; "and as for the other" |
| 0x23, 0x24 | has nothing to say about that (one of two) |
| 0x76 | does not know where that is |

**A line** is its text and its voice (`<stem>_<quote>` and a letter for each
line after the first). Some texts are not spoken:

- an empty line is skipped;
- `EMPTY` or `BLANK` is skipped, but for quotes 0x67 to 0x69, where the
  player is told the person has no answer (two wordings, alternating);
- `UNKNOWN` says quote 0x1c or 0x1d instead, evenly; `CLASSIFIED` says 0x1e;
- `SOUND` plays its voice with no text.

A quote marked as an alert is a notice with the alert sound, not speech.

A line lasts as long as its voice; with no voice, 2 seconds and 60 ms a
character. The next thing happens when it ends.

## 3. What a quote does

Reaching a quote's last line, its entries are taken in order:

| Entry | What it does |
| --- | --- |
| next | another quote follows: a fixed one, one at random of a range, or one said by another named person, who becomes the one spoken with |
| if facts | the same, if every story fact named has the value named; otherwise the next entry is tried |
| set fact | a story fact changes |
| mention | the party learns a topic (a person, place, item or other) to ask about |
| give item, take item, give gold, give experience | once this quote's lines are said: the party gets or loses it and is told |
| options, typed word | once its lines are said, the party must answer: one of the words listed, or a word typed |
| price | before any of its lines: the person asks a price (section 5) |
| disposition, faction | a named person's standing with the party, or a faction's, changes |
| party remark | a member says something about it |
| close | the conversation ends once what is queued has been said |
| close and resume | the same, and the person goes back to what they were doing, cool toward the party (25) |
| script label, world action, leave party | the person's body does something in the world |

The first *next*, *if facts* that holds, *options*, *typed word* or *close and
resume* ends the list: nothing after it is taken.

Things said and done queue up in order; what a quote gives is given after its
lines and before whatever follows it.

## 4. Standing

How a person stands toward the party is a number, 0 to 99, in three bands:
**hostile** up to 32, **neutral** up to 65, **friendly** above. A charm spell
on them adds its strength for as long as the conversation lasts; one turned to
the party's side counts as 100. One who belongs to a faction and is bound to a
monster, or whose faction is hostile, stands as the faction does.

A merchant, a voice-only speaker and one bound to a monster's script always
talk as friends.

## 5. What the party may do

Each is possible only when the person is not speaking and nothing is queued,
except answering, which is possible only when asked, and leaving, which is
always possible.

**Begin.** With a free hand, not fighting, within reach. The person turns to
the party. A hostile one says 0x18 and that is all. A neutral one says 2 and
the party may only charm, chat, give or leave until they are friendly. A
friendly one says 0 the first time (and the party learns their name as a
topic, and every member fit to fight practises Communication, 15) or 1 after.

**Ask about a topic** (friends only). The words are matched against the
person's topics: the whole text, then each pair of adjacent words, then each
word. The first match is said; with two different single-word matches the
second follows after 0x21; with more than two, 0x20. With none, 0x23 or 0x24.
A topic that matched is remembered. `join` asks them to join; `where is …`
asks after a person or place, 0x76 if unknown.

**Answer** an options or typed-word question. A word that is one of those
listed takes the quote's reply for that word, or its general reply; any other
word takes its reply for a wrong answer. With no reply to take, the question
stands.

**A price.** A quote with a price is not said until paid for, unless the
person was paid for it before (and its price is not one asked every time).
The person says 0x12, then the party is asked. The price is the quote's, less
the charm on the person in percent, less a five-hundredth for each point of
the party's best Communication, at least 1. Refused, the quote's decline
quotes are said. Accepted without the gold, 0x14. Accepted, the gold is paid
and the quote is said.

**Give an item.**
- To a friend: the quote their script has for that item, if any; they keep
  it if the script says so. Otherwise an item that must not be lost is
  refused (0x11); any other is taken with thanks (0x10).
- To one who is not: an item that must not be lost is refused (0x11); one
  they have no use for is refused (7). Otherwise they take it; if their
  script has a quote for it, that is said; if not, its worth is taken off
  what they still want (below) and they say 0x10 if that made them a friend,
  7 if not.

**Give gold.** All the party has asks for confirmation first. To a friend it
is simply taken (0x10). To one who is not, and who can be bought, it is taken
off what they still want; 0x10 if that made them a friend, 7 if not. One who
cannot be bought says 7 and takes nothing.

**What they still want** is the person's trade pool. A gift counts for its
worth and a fifth of it more for each hundred of the party's best
Communication. When the pool is used up, they become friendly (75) and the
pool is whole again.

**Charm, small talk.** Each once in twelve hours of game time for each
person. The change to their standing is the person's own figure for it, and a
fifth of it more for each hundred of the best Communication; a charm that
improves is scaled by the party's average level over the person's. The best
speaker practises (5 for a charm, 8 for small talk). Then they answer by how
they now stand: hostile 0x18 and the conversation ends; neutral 6 or 3;
friendly 4, or the greeting.

**Ask them to join.** No room, 0xc. Will not, 0xb. Joins, 0xd, and the
conversation ends once it is said ([levels.md](levels.md) says what joining
is).

**Trade.** Buying and selling are the merchant's rules (`npcs/trade.rs`); the
conversation only has to be idle.

**Leave.** What is being said is finished; nothing more is begun. The charm
on the person ends.

## 6. What ends it

The party leaves; a quote closes it; a hostile answer; the person joining; a
fight beginning; the party travelling or camping; the person no longer being
there. However it ends, nothing half-done is left: a price not answered is
not paid, an item offered and not taken is still the party's.

## 7. Seen in the original and not in this game

Read off the decompilation while writing this page. Slice 8 fixes each as
its act is rebuilt, confirming it against the decompilation first; the last
is not part of that slice.

- `where is` and `join` typed as topics are understood: `where is` looks a
  name up among the people and regions the person knows of (0x76 if they do
  not), by tables this game does not have yet.
- Leaving a friend, they say a farewell (0x5c), and after talking with one
  who is not a merchant a member may remark on it a second later.
- A prompt to turn the person hostile on purpose (they become cool, 25, say
  0x18 and go back to what they were doing).
- The story commands a quote can queue (a person walking off, a door
  opening, a henchman called, the endgame) are not carried out; a quote that
  needs one is refused before anything is paid or given.

## How this is built

The sections above are the rules. This says how the code is to be shaped
around them; where the code differs from it, the code is wrong.

**A conversation is an entity.** It names who is spoken with. What the person
still has to say is a queue on it. What has been said is its transcript.

**What it waits on is there to be seen, not a state to be switched on.** A
line being spoken is an entity of the conversation; while one exists nothing
else is said. A question put to the party (options, a typed word, a price,
giving everything) is a component on the conversation; while it is there only
an answer is taken. A conversation is idle when it has neither and its queue
is empty. There is no field saying which of these it is in.

**What the party does is an event at the conversation, with one observer
each.** The observer checks that the act is possible now, changes what the
rule says it changes, and queues what the person says. An act that is not
possible changes nothing; one the player should hear about is told as news.
Nothing is left on the conversation for a later system to pick up.

**Saying is one chain.** When something is queued and nothing is being said,
the next line is spoken; when the line ends (the screen says so), the next is.
A quote's entries are carried out as its last line begins, each by the event
that already exists for it (an item given or lost, practice, a faction
changing). No system looks at every conversation every frame.

**Trade and joining are their own rules.** The conversation asks; the answer
comes back as an event it observes. It holds no flag meaning "waiting for the
merchant".

**Endings are events too.** A fight, travel, camp or the person going ends
the conversation where that happens.
