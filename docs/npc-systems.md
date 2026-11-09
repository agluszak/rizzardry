# NPC systems: requirements and design

## Authority and scope

The current Rizzardry implementation and its tests are not specifications. Neither
its entity identities nor its body, lifetime, queue, scheduling or module boundaries
are requirements. Population, level transitions, death handling, party integration,
content decoding and presentation may all need changes. Existing interfaces do not
justify preserving an unsuitable model.

The behavioral evidence below comes from the reconstructed original-game source in
`../wizardry-8-decomp`. Function names identify the evidence; reconstruction comments
are not independent confirmation of behavior. Where evidence is incomplete, this
specification says so rather than treating the old implementation as a fallback.
Original memory layouts, global managers and runtime indices are not design mandates.

## Observed identity and lifetime behavior

1. `NPC Manager.cpp::ResetNpcStates` creates state for every database record with
   `monster_bound == 0` at reset. These records exist before a level places a monster.
2. `CreateNpcRuntimeNode` initializes recruitable character data, merchant inventory,
   social state and other record state together. Initializing a character only on
   first recruitment would change initialization timing and random-number consumption.
3. `BindNpcToMonster` finds an existing non-monster-bound record, but can create a new
   instance for a monster-bound definition. A definition identifier is therefore not
   universally an instance identifier. The current `(definition, map, placement)`
   tuple is not established by this evidence as a permanent instance identity.
4. `ReleaseNpcBinding` releases the script and world binding. Monster-bound records
   become unavailable; non-monster-bound state remains. Losing a physical binding
   does not by itself mean the same thing as death or loss of campaign state.
5. `RebindNpcLevelTriggers` binds merchant and voice-script records to named triggers.
   Such a speaker need not have a physical monster. A universal “living body in reach”
   admission rule would exclude this supported class of interaction.
6. `RecruitNpcIntoParty` copies retained character data into a party slot, transfers
   current monster conditions, removes the monster, and marks the NPC grouped.
7. `DismissNpcFromParty` saves the changed party character back to NPC state, removes
   membership, and normally creates a new monster near the party. It transfers
   conditions to that monster. This does not require a dormant monster entity.
8. `SaveNpcStates` stores record state, retained character data, and merchant stock.
   `LoadNpcStates` reconstructs records and bindings. Save identity need not be an ECS
   entity identifier and live physics/render objects are not the persistence model.
9. `UpdateNpcPartyMember` distinguishes dead, missing, placement failure and regional
   service behavior. `Health Stamina Mana.cpp` marks NPC state when its party
   character dies; `Magic Effects.cpp` clears that mark when the character is
   resurrected. Therefore “death is always irreversible” is not a valid general
   requirement. `RemoveCharacterFromParty` separately clears occupancy and formation.

These observations require distinct concepts: authored definition, runtime instance,
remembered campaign state, active world binding, party membership, and presentation.
They do **not** determine how many entities there are, or which object owns them.

## Conversation behavior

The evidence is `NPC Scripting.cpp::RunNpcScriptLine`, `ProcessNpcQuoteEntry`,
`QueueNpcScriptLine`, and `NPCInteractionSubscreen.cpp::EndNpcDialogueSession`.

* A speaker can be a world NPC, a grouped NPC, or a trigger/script speaker. Admission
  must validate the appropriate interaction source, not demand one universal body.
* Quotes have ordered subquotes and ordered entries. On reaching the final subquote,
  the original executes some entries immediately and queues others. A single eager
  “plan accepted at the first line” is not an established behavioral requirement.
* Facts, conditionals, item/gold/experience effects, keywords, options, speaker
  changes and closure have observable ordering. A conditional must observe fact
  assignments already executed before it. A speaker handoff can target a party NPC.
* Closing the dialogue panel and ending scripted playback are different operations.
  `EndNpcDialogueSession` resumes the world, clears monster charms and flushes notice
  lines. `FlushPendingNoticeLines` only displays/deletes already prepared text notices;
  it does not execute the script-effect queue. The distinct `ProcessMessageBoxQueue`
  executes queued script lines and entries. Closing a panel is not evidence for
  immediately awarding every future effect.
* The exact behavior of queued effects during travel, combat interruption and camp
  needs the queue/caller paths traced. The discarded implementation's “drain all
  accepted effects before travel” policy must not be imported without that evidence.
* Unsupported mandatory operations must report their location and stop execution.
  They must not be silently discarded or cause a payment to be consumed for a
  response the interpreter cannot execute.
* UI text, audio duration and layout are presentation. Gameplay controls which
  response is legal and when a choice or payment is accepted. Completion must refer
  to the active line, so a stale acknowledgment cannot advance a later line.

## Trade behavior

Evidence: `npc_items.cpp::PopulateNpcStock`, `MaintainNpcStock`, `RestockNpcItems`,
`CalculateTradeStackPrice`, `SellItemToNpc`, and their interaction-screen callers.

Inventory for sale is distinct from a recruitable character's equipment. Population
uses authored stock rules and item difficulty. Persistent replenishment and daily
maintenance are separate. Initial stock is created with NPC runtime state, not as a
side effect of opening the conversation. Pricing incorporates Communication,
merchant factors, identification and ammunition quantities. Preserve arithmetic
rounding only after verifying the source expressions.

Modern transaction requirements: quoting does not mutate inventory; confirmation
checks current source identity, ownership, quantity, price, funds and capacity before
changing anything. A replayed confirmation cannot repeat the transfer. Stable quote
identity and invalidation are design decisions, not claims about retail internals.

## Recruitment behavior

Evidence: `NPC Manager.cpp::CanNpcJoinParty`, `RecruitNpcIntoParty`,
`DismissNpcFromParty`, `ReturnDismissedNpcItems`, `UpdateNpcPartyMember`;
`GameplayCode.cpp::AddCharacterToParty` and `RemoveCharacterFromParty`.

Admission depends on a recruitable template, available RPC capacity, region service,
authored story conditions and eligible party members' average level. Character
progress survives leaving and returning. The first-character initialization point
is runtime-state creation. Recruitment and dismissal exchange conditions with the
world representation, not necessarily its health or equipment representation.

Dismissal returns items the NPC does not want and can relinquish; overflow handling
belongs to inventory placement. `NpcWantsItem` matches explicit item IDs and the
shared unidentified-name category 0x83. While grouped, it also checks the number
already carried. A static wanted-ID list alone does not express that rule.
Formation must reflect membership, but component hooks are only one possible design.
Placement failure and scripted forced departure require explicit outcomes.

## Implemented ownership and scheduling

* An actor entity owns social state, retained recruit data and merchant stock. `ActorId`
  selects authored content; the entity identifies the runtime instance. Named actors
  are created when a campaign destination and catalogs are available, using the proposed
  starting party's level. Monster-bound records are created for placed instances. The
  registry remembers their placement association; that association is not a definition ID.
* A physical monster belongs to its level. `NpcBody` relates it to its actor without
  making actor lifetime own body lifetime. Departure records a snapshot and destroys
  the level's entities. Arrival consumes the dormant snapshot into a fresh body. There
  is no disabled physics entity and no retained live snapshot alongside an active body.
* An unrecruited character is a `StoredCharacter` value, not an active `Character`
  component. Joining moves that profile into the character systems and establishes
  `PartyMember`; dismissing moves it back and creates a fresh monster with transferred
  conditions. Membership owns formation updates. Death and resurrection remain part
  of character condition handling; fallen NPC members retire outside combat.
* Current physical bodies, party membership and level trigger bindings are distinct
  interaction endpoints. Trigger speakers are bound by the source name in the level
  definition and do not require a physical body.
* A conversation entity owns its continuation, pending question and transcript.
  Speech entities belong to it; acknowledgments identify the exact speech entity.
  Ordinary systems accept input, run domain transactions, then advance playback.
  One subquote advances per update. Final-subquote entries execute in script order;
  immediate fact assignments are visible to subsequent predicates. Reward delivery
  follows interpretation in the same update. There is no exclusive-world interpreter
  or cached-system orchestration in these gameplay modules.
* Trade owns stock maintenance, read-only pricing and settlement. Its quote contains
  economic identity and a stock revision, not a conversation handle. Settlement
  recalculates the price and validates the whole transfer before mutation. Conversation
  adapts the result to UI feedback. Recruitment likewise owns admission and membership
  transfer; it does not reach into conversation state.
* Story facts and notebook entries belong to campaign state, independently of any
  conversation or physical binding. Unsupported derived predicates and consequences
  fail explicitly. A quote is checked for unsupported operations before payment or
  item surrender; support checking does not execute its conditions ahead of time.

## Explicit implementation limits

This rewrite does not complete the original game's missing story systems. Scripted
forced departure, monster-script labels, unaudited fact predicates/consequences and
unknown opcodes stop with a location-bearing diagnostic. Separate RPC-script selection
for grouped speakers is not yet ported; they currently use the NPC script. Save/load remains outside
Rizzardry's current capabilities. Initial stock and character initialization follow
the observed lifetime; identical original-game random streams are not claimed.

Travel, combat, camp and loss of an interaction endpoint cancel a conversation's
unexecuted continuation. They do not award future script rewards. This is an explicit
implementation policy pending further tracing of the original queue callers, not a
claim that the original game uses the same cancellation rule. Recruitment commits
when its request is accepted, before the joining response finishes speaking.

## Replacement and validation

The old gameplay implementation and its dedicated tests have been deleted. Do not
restore or mechanically translate them. Do not preserve their architecture merely
to satisfy existing callers; adapt callers to the design once derived.

The replacement must compile and pass formatting and denied-warning Clippy. Validate
observable workflows against original-game evidence and a running retail-data game.
Do not add tests that merely encode the new implementation's structure. Report
unresolved behavior and actual validation limits instead of calling compilation,
screenshots or test counts proof of architecture quality.
