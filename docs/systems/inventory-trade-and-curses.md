# Inventory, trade, identification and curses

## Ownership

An inventory item is an owned value. A character owns equipment/backpack values;
`PartyInventory` owns the shared pool, purse and held item; a container/pile owns
its contents. A conversation can temporarily own an offered item. Selection and
view rows own references, never copies of live items.

```rust
struct ItemKnowledge { identity: bool, spell: bool, curse: bool }
struct ItemInstance {
    id: ItemInstanceId,
    definition: ItemDefId,
    quantity: u16,
    charges: u16,
    knowledge: ItemKnowledge,
    curse_released: bool,
}
struct ItemRef { owner: ItemOwner, id: ItemInstanceId }
```

This sketch does not prescribe integer widths or the final quantity encoding.
Definitions specify stacking/use rules; constructors validate the instance.
An identity is useful for queued uses and item-targeted spells without making
each item an entity. Splits create a new identity; merges retain the destination
identity and retire the source. Any choice referring to a retired source is
invalidated explicitly, not silently redirected by vector index.

Common operations are `pick_up`, `place`, `move_item`, `equip_item`, `consume_use`,
`remove_quantity` and `receive_items`. Each returns the actual result/remainder.
Merging can succeed partially. Identification or acquisition side effects may
already have occurred even if placement cannot fully complete. No blanket
transaction rollback is implied.

Whizzardry's [Inventory](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/inventory.h)
and [implementation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/inventory.cpp)
already centralize transfers and held state. The new design keeps complete
operations while removing record-pointer repair, view-owned mutations and a
catch-all game dependency. A full selected item does not have to become an entity
to survive sorting.

## Cursor transfer

1. Resolve the source and check removal permission, including curses.
2. Move the requested value/quantity into held state, retaining enough provenance
   to attempt a return. The source no longer owns that quantity.
3. Hover/preview reads eligibility without mutating either inventory.
4. Placement calls the normal destination operation, including partial stacking.
5. Any remainder stays held. Closing a screen cannot delete it.
6. Cancellation tries the permitted return/receipt path; it does not recreate a
   source slot that another completed operation now owns.

Targeting an item for combat is different: selection leaves it in inventory
until an accepted execution consumes it. See [combat items](combat-items-and-magic.md).

## Trade: original versus modern implementation

The original uses persistent NPC stock and temporary dialogue selections.
Whizzardry now has a typed `W8TradeSelection` and a centralized
[`CompleteNpcTrade`](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L443-L650),
with purchase receipt coordinated through inventory. That is a useful seam,
though selection still depends on source slots and code still reaches globals.

Merchant stock is deliberately not ordinary party inventory. The original
[`AddNpcItemFromInstance`](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L368-L416)
uses the item's definition and quantity rather than copying its instance
metadata. New stock templates are identified and initialized with full charges;
existing rows keep their template. Selling and buying back can therefore change
knowledge/charges. Preserving the exact sold instance would be a gameplay change.

| Data | Owner |
| --- | --- |
| Accepted classes, price factors, stock-generation rules | `MerchantDef` |
| Lots, quantities, stock templates, delayed availability and restock time | NPC `MerchantStock` |
| Relevant theft/topic memory | NPC state |
| Player items/gold | Inventory owners described above |
| Mode, source, selected item/lot, quantity and filters | Temporary `TradeUi` in conversation state |

### Open, select and quote

| Step | Responsibility | Mutation |
| --- | --- | --- |
| Start merchant conversation | `begin_conversation` | Maintain stock against game time; initialize session |
| Choose buy/sell and source | `update_trade_selection` | Temporary selection only |
| Choose quantity | `update_trade_quantity` | Temporary quantity; ordinary stacks start at one, ammunition uses whole bundle |
| Display quote | `quote_trade` | Pure calculation |
| Confirm | `execute_trade` | Revalidate current references/quantity and complete the operation |
| Refresh or close | UI/session systems | Present result; completed exchanges remain committed |

Sales select a backpack or shared inventory, not still-equipped items. Ordinary
quantity adjustment and ammunition bundling are separate UI rules. There is no
cart settled when the dialogue closes.

The original [price function](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L735-L773)
uses the best party Communication skill, not the selected speaker. Preserve its
buy/sell factors, rounding and unidentified quarter-price rule (minimum one gold).
It does not use disposition in that function. Whizzardry retains that formula.
Keep it a testable calculation over explicit inputs.

### Sale commit

`trade::sell` resolves the actual item/quantity, validates discard and accepted
class rules, and checks RPC refusal to surrender a wanted possession. It computes
proceeds, including the merchant's remembered stolen-definition payment rule,
then removes the source quantity, updates normalized stock and credits gold in
one deliberate operation. Return the refusal/completion and presentation details.

The [original sale path](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L7521-L7642)
does not require a sufficient merchant purse. Do not invent that rule because
an NPC record happens to contain gold. Theft memory refers to item definitions,
so it is not a full ownership-history system.

### Purchase commit

`trade::buy` validates stock availability and the price of the full requested
quantity. Insufficient funds refuses it rather than reducing the order. Construct
identified items from the stock template, split them at stack limits and determine
their destinations. Apply stock reduction, payment and delivery together before
presenting success.

The original [purchase](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L775-L841)
uses shared-inventory insertion with stack merging and overflow. Ordinary excess
can drop to the world; protected items need explicit held-item handling. Preserve
unplaced values. Whizzardry's `receiveAll` coordinates destinations before payment;
the Rust version can express that as one readable operation, without a reusable
transaction/event framework.

Closing conversation discards selections and restores any appropriately held
item; it does not undo completed transactions. Save stock and purse, not view
rows or the price text.

## Identification

Identity knowledge, spell knowledge and curse knowledge are separate from the
actual immutable definition. Knowledge influences both displayed information and
rules such as item-use difficulty and trade pricing.

### Discovery and inspection

Identification is explicitly called at discovery/interaction points. It can run
when a world item is found, before acquisition, as shown in
[`Search.cpp`](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Search.cpp#L259-L272).
Container exposure, held-item delivery and inventory inspection also call it.
Low-level insertion is not a universal identification hook.

`inspect_item` resolves a value, calls `try_party_identification` where the
interaction requires it, and opens the information view. For an eligible member:

```text
strength = floor(Artifacts / 6)
strength >= item difficulty: reveal identity and associated details
strength >= item difficulty + 3: also reveal curse status
success margin 0..2: apply the recovered Artifacts practice rule
```

The success comparison is deterministic; skill practice has its own machinery.
The [party helper](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L2048-L2071)
initially skips an identified item, otherwise continues eligible members after a
first success. This differs from trade's best-skill calculation. Stronger curse
discovery and practice can depend on those later attempts. The
[per-member check](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L2535-L2571)
must not run every frame or on every repaint.

### Identify Item spell/item

Select caster or spell-bearing consumable, then an `ItemRef`. A target may have
unknown identity or known identity with still-unknown curse information. Validate
and execute through the normal magic/use pipeline, pay its casting cost once,
then resolve knowledge according to effect strength. Successful magical
identification uses the percentage-adjusted `3 * strength` comparison and reveals
curse status too. It does not remove a curse.

Consuming a scroll in the target's packed inventory may shift every following
index. Item identity avoids the pointer/index repair found in the old UI path.
There is no separate cost per knowledge flag revealed. Do not add paid merchant
identification as established behavior: the inspected paths establish buy-back
identification, not a general paid service.

## Cursed equipment

Four facts remain separate: the definition is cursed; the party knows that; the
item is actively equipped; and Remove Curse may have released it for removal.
The decomp names are misleading: `bound` behaves as curse knowledge, while
`bind_announced` behaves as release. The
[information dialog](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/dialog_code/AssayDialog.cpp#L256-L272)
confirms that distinction. Whizzardry's consumers retain the same important
predicates; do not preserve the misleading field names.

### Equip and attempt removal

`equip_item` validates slot/user rules and the outgoing item, moves the value and
updates derived equipment effects. Voluntary removal is prohibited when all are true:

- Definition is cursed.
- The item occupies active equipment, rather than alternate-weapon storage.
- Its curse has not been released.
- The wearer is alive.

Knowledge is not part of that predicate. An unknown curse already restricts
removal. A blocked removal can reveal it and show a notice; the next click is
still blocked. A cursed item in a backpack remains movable. See
[removal checks](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L1293-L1302)
and [discovery on blocked removal](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/ReviewCharacterScreen.cpp#L2109-L2122).

All voluntary removal paths call the same rule, including drag, swap and
preparation to sell. Forced removal of newly unusable equipment after a
profession/stat change is a separate source-backed operation; see
[progression and equipment settlement](progression-and-map.md). No `Curse` entity
or stored wearer ID is necessary for this restriction.

### Remove Curse and Hex

Remove Curse resolves through shared magic:

1. Attempt to lift the target character's separate Hexed condition.
2. Examine equipment; compare cursed definitions' difficulty with adjusted
   `3 * effect strength`.
3. Set release on qualifying values and report none/partial/all release.
4. Charge through normal casting, not per item released.

Equipment release has no countdown in this implementation. The normal move
clears it, so removing and re-equipping the item restores the restriction. The
definition remains cursed. Release does not itself need to identify the item.
Sources: [release](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L2132-L2156),
[move reset](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L2770-L2806).

Hex applies an actor condition through resistance/chance/duration rules. Remove
Curse can address it and can apply Hexed on backfire. This is not a mutation of
an inventory definition. See the
[spell dispatch](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L3436-L3447).
Enchanting an otherwise uncursed item with a new curse would be an additional
feature, not implied by these original paths.

## Ownership boundaries and checks

`inventory` owns movement, equipment and knowledge/release helpers; `trade` owns
prices/stock/exchanges; `magic` invokes item/condition effects; UI owns selection.
Persist item values and knowledge, NPC stock, purse and released equipment state.
Do not save raw references or reconstruct gameplay by replaying UI clicks.

Useful checks: partial merge retains the remainder; last-unit use invalidates
choices; selling an unidentified/partly charged item follows stock normalization;
purchase overflow preserves all paid-for items; an RPC can refuse surrender;
inspection does not repeatedly train an identified item; identifying a scroll's
neighbor still targets the same item after scroll consumption; unknown cursed
equipment blocks removal; released equipment moves once and binds again when
re-equipped; Hex and equipment curses remain independent.
