# NPC conversations and trading

Retail species bind through their NPC database identity, never the serialized
runtime slot. `NpcCatalog`, `FactCatalog` and `DialogueScript` decode the user's
installation; gameplay contains operation semantics, not Burz's words, keywords,
record number, stock list or price factors.

A named NPC retains one actor entity through travel. Repeatable monster-bound
NPCs also key identity by authored map/placement. An active actor has `Monster`
and spatial components; departure moves its world-role value and placement into
its dormant state and removes embodiment before despawning the map. Level records
exclude NPC copies. Neither stock nor NPC memory belongs to a view.

`Conversation` owns authored continuation and identified speech/choice/payment
waits. It retains source order: last-subquote effects precede that subquote;
continuations wait for playback and its 700 ms hold. Missing audio uses the
source text-duration fallback. Information-price acceptance pays and remembers
the quote on that NPC. Campaign facts and learned topic mentions survive closure.
Fact IDs and journal text come from `FACT.DBS`; supported typed handlers resolve
its symbolic names. Unknown predicates, assignments, operations and unsupported
response modes block with the originating quote/entry instead of advancing.

Merchant stock consists of identified templates/lots, separate from personal
gear. Generation and maintenance use decoded rules, explicit gameplay RNG and
the game clock. Sales normalize merchandise rather than preserving the sold
instance's charges/knowledge. Quotation is a draft; confirmation revalidates its
conversation revision, source identity, quantity, price and funds. Delivery is
planned with existing inventory insertion before stock, purse and receipt commit.
A full pool refuses the purchase without partial merges or payment. This is a
deliberate bounded-destination departure from retail's world-drop overflow.

## Evidence

Original-binary reconstruction at `wizardry-8-decomp` commit
`075cbd0b853c4670e94f6ae4fe48d53b52b5e819`:

- `npc_script_file.cpp`: packed NSF records and UTF-16 dialogue/narrow keywords.
- `NPC Scripting.cpp`: `RunNpcScriptLine`, `FindNpcReplyQuote`, response predicates,
  price interception and speech completion hold.
- `NPCInteractionSubscreen.cpp`: information-price quotation and acceptance.
- `npc_items.cpp`: initial stock, maintenance, trade factors and normalization.
- `PC Item.cpp`: ammunition's rounded bundle value.
- `NPC Manager.cpp`: NPC creation, binding and disposition thresholds.

The retail acceptance case is Burz's greeting and Monastery/Arnika branch,
including both replies and its retained fact. Tests may assert known content
identifiers; runtime content selection does not embed them.

## Remaining gaps

Other fact semantics, NPC world actions, alert/sentinel playback, services, theft,
NPC death, recruitment and scene animation waits remain explicit gaps. The
special restock gate for item 0x1fc refuses maintenance until its fact semantics
are ported. The dialogue view is a functional native layout, not retail screen
parity; its editable field uses native text rendering. Disk saves and off-map
condition catch-up are outside this slice.

Useful focused checks:

```sh
cargo nextest run --locked -E 'test(/npcs::/)'
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo nextest run --locked --run-ignored all -E 'test(/burz/)'
```
