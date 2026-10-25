# Crafting, commissions, and NPC services

## Status and decisions

This plan compares [wizardry-8-decomp at 075cbd0](https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819) with [whizzardry-8 at 9c433ec](https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873). The behavior below comes from those pinned sources. The Rust owners and operations are proposals.

There are three distinct creation paths:

1. A character combines two inventory items according to an item-database recipe.
2. An alchemist produces a potion through the profession's resting/cooldown rule.
3. An NPC accepts quest materials and eventually offers a commissioned product through stock and dialogue.

Use ordinary functions and small persistent records for these paths. They do not justify a general crafting graph, inventory transaction framework, job-entity hierarchy, or service scripting engine. Item instances and ingredient counts remain values, with stable IDs for references that must survive inventory moves.

## What the two implementations establish

| Concern | Original reconstruction | Current modernization | Rewrite implication |
|---|---|---|---|
| Manual recipes | MergeItems scans item definitions for the two inputs in either order, checks identification and a skill threshold, consumes a quantity, then creates the result. [D1] | Inventory::combineHeld owns the two-slot mutation and returns a refusal value. [M1] | Keep this a complete inventory operation with explicit input IDs and actor. |
| Partial consumption | Both stack inputs consume the smaller count; other combinations consume one unit of a stack and the nonstack input. [D1] | Computes remaining values first, then assigns both destinations. [M1] | Compute quantities and destinations before mutation; retain leftovers. |
| Practice and result placement | Skill practice precedes product creation; some products replace the held item. [D1] | Practice follows product creation; when the target becomes empty, the product replaces it. [M1] | Use the original ordering and placement as the starting baseline; verify these differences with representative fixtures. |
| Automatic brewing | A profession-level table and cooldown produce a potion when the rest gate allows it. [D2] | Same table and gate with Game-owned inventory/RNG calls. [M2] | Separate this from ingredient recipes and ordinary Alchemy skill checks. |
| Commission output | Ingredient-complete facts add delayed NPC stock; maturation sets completion facts. [D3, D4] | Same delayed stock rule; NPC state has an owned stock vector and explicit available_at. [M3] | Keep pending products with the persistent NPC, including while its map is unloaded. |
| Paid dialogue and purchase | Price-check replies deduct gold before running the accepted script line. Ordinary stock purchase is a separate path. [D5] | Dialogue has an explicit pending price check; stock purchase revalidates and waits for a destination before payment/stock removal. [M4, M5] | Retain the separation between accepting a service offer and buying an available product. |

Retail item and NPC script records are not present in these repositories. We can recover recipe fields, algorithms, commission product IDs, and clocks. We cannot claim an exhaustive named recipe catalogue, exact donated-item acceptance tree, dialogue text, repeat-order policy, or each paid service's effects without those records.

## Data and ownership

| Data | Rust owner | Notes |
|---|---|---|
| CombineRecipe | Immutable content value | Two ItemKindIds, output ItemKindId, optional SkillRequirement. Input order is interchangeable. |
| ItemInstance | Existing inventory values | Keep quantity, charges, identification, and stable instance identity; do not create ingredient entities. |
| Actor skill/profession and brew cooldown | Existing character components/values | Manual merge reads the required skill; automatic brewing reads alchemist profession level and stores its cooldown in AgingTicks. |
| Pending combine input | UI selection | ActorId and two ItemInstanceIds, plus requested locations/revisions if necessary. Revalidate on submit. |
| Commission definition | Content value referenced by its named NPC | Accepted ingredient requirements, output, delay, dialogue/fact effects, repeat rules once decoded. |
| Commission progress | Component/value on the campaign-lived NPC actor | Materials already accepted and the link to its resulting stock entry. This outlives the NPC's active world body. |
| MerchantStock | Component on that same NPC actor | Each row owns its item value, quantity, stable stock ID, and optional ready_at. Pending output is stored exactly once here. |
| Merchant maintenance deadlines | NPC stock state | Use campaign time, not a Bevy Timer tied to map presence. |
| Pending service offer | Active dialogue state | NPC ID, authored service/quote ID, chosen target if any, displayed price, and required conditions. |
| Campaign clock and RNG | Existing game resources | Recipes, practice, stock, and brewing call the agreed rule functions; UI previews do not advance them. |

A useful commission progression is Collecting with received ingredient counts, then Stocked with the stable stock-entry ID, then Purchased/closed if its content requires that distinction. While Stocked, readiness is derived from that row's availability state. Do not also keep a second timer, a second product item, and a parallel ready boolean in the commission record.

The NPC actor remains in campaign state off-map; its renderer, collider, and movement body need not. Dormant map snapshots reference the NPC's ActorId and do not copy its stock or commission progress. PartyRoster owns membership and formation; PartyInventory owns the shared pool, purse, and held item. Neither belongs to the spatial party body.

## Walkthrough: manual combination

### Recipe lookup and rejection

The original recipe is encoded on the output item record: merge_component_a, merge_component_b, merge_skill, and merge_skill_level. The first matching record in database order wins. There is no random craft-success roll in this path. The gate is identification plus a skill threshold. Gadget construction and ingredient-based alchemy can therefore use the same small combine operation; content tells it which skill is required. [D1, M1]

The import step should build an index by the unordered pair of input item kinds. Detect duplicate recipes instead of silently changing their precedence. Either preserve original first-match order or resolve a duplicate deliberately after inspecting the actual data.

| Step | Proposed operation | Rejection or commit |
|---|---|---|
| Select two items | UI emits CombineItems with actor and stable input IDs | Selection alone changes nothing. Do not remove an item merely to preview a recipe. |
| Resolve live inputs | combine_items looks them up in the permitted inventory locations | Reject missing/stale inputs, identical instance IDs, missing actor, or incompatible location before any mutation. |
| Check identification | Read both item values | Both must be identified for the recipe path. Failure consumes nothing. |
| Find recipe and skill gate | lookup_combine_recipe and check_recipe_skill | Accept either input order. Insufficient skill does not consume recipe ingredients or practice; evaluate the original same-kind stacking fallback separately. |
| Decide quantity and output location | Pure quantity/location calculation | Validate the output definition and room for all input remainders/product before any practice, item creation, or consumption. Follow the original slot-selection rule unless a fixture-backed decision changes it. |
| Resolve practice before item creation | Existing skill-practice function, within the accepted combine_items operation | Follow the original practice-before-creation RNG order. Preflight has already eliminated content/placement refusals; when the recipe names a skill, practice runs once for the accepted recipe and never on redraw or a rejected request. |
| Construct result | Existing make_item rule, after practice | Product defaults, identification, charges, and random draws follow the original creation order. Do not insert a preview draw or move practice after this step. |
| Commit combination | Replace the two source values with remainders/product | Practice, creation, and slot updates complete within one synchronous game operation. No frame or save may observe consumed inputs without the output or a separately committed practice-only outcome. |
| Notify | Return the completed result to UI/presentation | Display the actual product, remainders, and skill changes; this step consumes no gameplay randomness. |

When both inputs are stacks, the amount is min(left count, right count). If the output stacks, its count is that amount. If either input is nonstackable, combine one recipe unit: consume the nonstack value and one unit from any stack input. Preserve the original quantity semantics rather than treating charges as interchangeable stack counts.

Preserve the original held/target placement rule as well. For two stacks, the product takes the smaller stack's location; equal counts choose the target location. For every other combination, the product takes the target location if the held input is a stack, and the held location otherwise. The remaining input retains its own location. Stable item IDs replace pointer arithmetic, but do not change which location receives the product. [D1]

For example, a stack-output recipe with ingredient counts 7 and 3 creates three units and leaves four units of the larger input. This is an arithmetic example, not an invented named retail recipe. If an imported recipe has a nonstack output with two stack inputs, the observed implementation still consumes the calculated input quantity but creates one product; validate such records explicitly instead of silently introducing a new yield rule. [D1, M1]

### Stacking and refilling are separate operations

When no recipe succeeds, the original has a same-kind stack fallback. Ordinary stack merging moves as many units as fit and leaves a remainder. Pouring uses/charges is another path, governed by quantity kind and maximum capacity. Do not call all three operations craft, and do not apply recipe practice or ingredient rules to a simple stack merge. [D1, M1]

### Differences to settle explicitly

The modernization is a useful ownership example, but it is not a byte-for-byte behavior oracle:

- Original MergeItems practices the skill before calling item creation; modern combineHeld practices afterward. This can change subsequent shared RNG draws even when the visible recipe outcome is the same.
- With two nonstack ingredients, the original chooses the held slot for the product; the modern operation puts it into the newly emptied target. Some mixed one-unit cases also differ.
- If a matching recipe fails its skill threshold and both inputs have the same item kind, the original can still fall back to stacking; modern code immediately returns the skill refusal. Whether retail data contains such a recipe needs checking.

The starting baseline follows the original reconstruction: practice precedes make_item after pure preflight has established that the operation can finish. Preserve ingredient quantities, threshold, practice amount, and that gameplay RNG order. Product-slot selection and same-kind fallback also start from the original rules; representative fixtures must establish their observed behavior before a different native rule is chosen. The modern implementation identifies possible simplifications, but this plan does not silently adopt its target-first placement or practice-after-creation order. Any departure belongs in the documented behavior decisions and needs no architectural layer change.

## Automatic alchemist brewing

This feature is not an ordinary combine recipe. The source checks that the character is present, below the unconscious condition threshold, and has the potion-making trait. It filters a fixed table using alchemist profession level, picks an eligible entry randomly, makes one potion, attempts to store it, and resets a cooldown to 0x168: represent that as AgingTicks(360). The aging parameter misleadingly named minutes counts two-minute world-time units; it is not a count of individual game minutes. See the [time and aging unit contract](time-rest-and-conditions.md). [D2, M2]

The time driver has a specific split: while the camping/surprise-possible gate is active, a zero cooldown permits brewing; otherwise delivered aging ticks reduce the cooldown. Do not turn this into a wall-clock timer, a roll every frame, or a loop that awards many potions during one long rest. Camping enables that gate in the modern source. NPC timer conversion is a separate rule and does not change the brew cooldown's unit. [D2, M2]

A small maybe_brew_potion function, called at the established rest/time boundary, should validate the trait/condition/cooldown, choose a product, and place it through inventory. It then commits the cooldown and emits a notice. Full-inventory handling must use the project's explicit overflow rule and must not lose the value or repeatedly reroll until something fits. The observed brewing functions do not inspect the store helper's result before rearming; verify retail overflow behavior before claiming exact equivalence.

## Walkthrough: Antone and Ferro commissions

### What the source proves

A positive ingredient-complete fact schedules one output in NPC stock with a delay of 0x15180, or 86,400 campaign-clock seconds. The modern function stores available_at = world_clock + delay. The product exists as stock before it becomes available. [D3, D4, M3]

| NPC | Ingredient-complete fact | Product item ID | Maturation fact |
|---|---|---:|---|
| Antone, kind 3 | ANTONE_ZYNARYX_ALL_INGREDIENTS | 0x28a | ZYNARYX_PLATE_COMPLETE |
| Antone | ANTONE_STEELHIDE_ALL_INGREDIENTS | 0x28b | STEELHIDE_COMPLETE |
| Antone | ANTONE_FEATHERWEIGHT_ALL_INGREDIENTS | 0x28c | FEATHERWEIGHT_COMPLETE |
| Antone | ANTONE_BEASTSLAYER_ALL_INGREDIENTS | 0x28d | BEASTSLAYER_COMPLETE |
| Antone | ANTONE_EBON_STAFF_ALL_INGREDIENTS | 0x28e | EBON_STAFF_COMPLETE |
| Ferro, kind 0x39 | FERRO_MIRROR_ARMOR_ALL_INGREDIENTS | 500 / 0x1f4 | MIRROR_ARMOR_COMPLETE |
| Ferro | FERRO_IVORY_BLADE_ALL_INGREDIENTS | 0x1f5 | IVORY_BLADE_COMPLETE |
| Ferro | FERRO_VAMPIRE_CHAIN_ALL_INGREDIENTS | 0x1f8 | VAMPIRE_CHAIN_COMPLETE |

All identifiers above have the W8_FACT_ prefix in source. Maturing Ferro's products also clears the corresponding ALL_INGREDIENTS fact; Antone's cases do not clear it in this function. A Fuzzfas potion uses the same delayed-stock mechanism with its own completion facts. These differences should live in the few commission definitions/handlers, not disappear behind a universal completion policy. [D3, D4, M3]

MatureNpcDelayedItems uses a strict comparison: a scheduled row matures when world_clock is greater than available_at, not at equality. It then clears the timestamp and sets the appropriate completion facts. Ordinary stock views exclude nonzero availability timestamps. Refresh on entering relevant NPC interaction evaluates stock, so the actor does not need to remain physically spawned while the party waits. [D4, M3]

### Proposed Rust sequence

| Step | Owner/function | Observable and irreversible effects |
|---|---|---|
| Discuss a commission | Dialogue reads the commission definition and current progress | Present requirements from content. No item transfer occurs merely because a requirement is mentioned. |
| Offer an ingredient | accept_commission_material resolves NPC, item, quantity, and recipe | Reject wrong/duplicate/unwanted material according to decoded script rules. A rejected offer keeps the player's item. |
| Accept material | Same function updates inventory and NPC progress | Consume/transfer exactly the accepted amount and record receipt together. Closing the conversation afterwards does not return donated material. |
| Complete requirements | start_commission called by that named dialogue/game rule | Create one pending MerchantStock row with a stable StockId and ready_at; link the commission to it. Record authored facts once. |
| Wait/travel/rest | Campaign clock advances through existing time systems | No visible crafting entity or per-frame off-map NPC system is required. |
| Reopen dialogue/stock | refresh_npc_services checks due stock before displaying availability | Mature eligible rows once, retain their item values, clear readiness gating, and apply specific completion facts. |
| Buy available output | Existing purchase_stock operation | Revalidate the row, price, gold, and destination. Move the item and settle payment/stock together. |
| Continue/repeat | Named commission content | Preserve repeatable versus one-time rules after retail scripts are decoded. Do not infer repeatability from an ingredient-complete fact alone. |

Use a dedicated stock row for each accepted commission instead of merging pending work into another already sellable row of the same item. In both C++ versions, AddNpcItemWithDelay first uses the ordinary add-stock helper and then changes that entry's timestamp; a merge can therefore couple otherwise separate stock. The rewrite can represent the intended job independently without preserving that incidental storage coupling.

Idempotence belongs to the commission operation. The original SetFact calls its side-effect handler even when a fact is assigned the same value again; simply rerunning the ingredient-complete handler on load could enqueue another item. Restore commission and stock values directly. Do not reconstruct pending work by replaying story effects. [D7]

The actual scripts are needed to establish which ingredient is removed at which dialogue branch, whether partial deliveries are accepted, prices/discount rules for the finished item, and repeated-order behavior. The presence of individual HAS_INGREDIENT facts is evidence that material receipts are tracked; it does not provide the complete acceptance logic. [D8]

## Paid services and ordinary stock

Keep authored service offers separate from the stock sale of a finished commission. The source price-check path stores a pending price derived from the authored base amount, NPC charm, and the party's Communication skill. On an affirmative reply, it rechecks gold, spends it, optionally records a fact, and executes the accepted quote. Insufficient gold uses a refusal branch; declining executes decline actions. The accepted quote's actual item/condition/story effects remain script content. [D5, M4]

In Rust, resolve_service_offer should revalidate the provider, target, conditions, and displayed offer before charging. For an immediate service, payment and the authored gameplay effect occur in one accepted operation; its voice/animation may follow. For a deliberately delayed service, payment creates the specific persistent pending result before the UI closes. A dismissed draft offer does not charge. An accepted service is not refunded because its speech was skipped.

Do not infer a paid healing/repair API from the C++ layout name SERVICES: the modern screen's named service buttons include player-cast Charm and Mindread. Import actual NPC service actions before naming additional operations. [M7]

Ordinary merchant stock has separate replenishment and decay clocks, with rule flags, party-level chance bands, and quantity jitter. Pending delayed entries are excluded from ordinary decay. The source uses roughly half-day and day thresholds, and performs stock maintenance before delayed maturation. Preserve the evaluation ordering when a due commission and a restock coincide; do not replace the whole process with “refill merchant every day.” [D4, M3]

For a finished commission purchase, the modernization provides a useful concrete invariant: stock and gold change only after every bought value has a defined destination. Its trade selection also validates expected contents, preventing a stale UI row from buying a different item after a sort or removal. Use stable stock/item IDs in the rewrite to enforce that same practical constraint without copying the current framework. [M5]

## Save, cancellation, and timing

- Manual selection and previews are UI drafts. Cancelling them does not consume ingredients or practice a skill.
- A combination is synchronous gameplay work. Save before or after its complete mutation, never between consumption and product placement.
- Save alchemist cooldown in aging ticks with the character and advance it through the established time gate. Loading must not generate a potion merely because a UI screen is rebuilt.
- Save accepted commission materials, the one pending/output stock row, ready_at, current stock quantity, and relevant story state on the NPC's single persistent record.
- An immediate service effect commits before another operation can observe its state. Manual saves during active dialogue follow the ordinary save refusal policy; there is no service-specific deferred save queue. A completed payment or material donation survives closing dialogue. The narrow deferral for an already accepted timed lock/trap attempt does not imply support for arbitrary in-flight service or scene saves.
- Loading or revisiting an NPC evaluates readiness from saved campaign time without restarting its delay or replaying an ingredient-complete fact.
- No stock, commission item, or material receipt is copied into a dormant map snapshot alongside the persistent NPC record.

No raw Bevy Entity, UI row index, audio handle, or wall-clock instant is part of this save model.

## Module dependencies

- content imports recipe fields, commission definitions, item defaults, stock rules, and NPC service branches.
- game/inventory owns combine_items, stack/refill operations, and placement of the resulting values.
- game/characters owns skill practice, profession traits, and brew cooldown values; its time update calls the brewing operation at the defined boundary.
- game/story or its NPC submodule owns accepted commission progress and authored dialogue effects.
- game/trade owns stock maintenance, readiness, and purchases; game/inventory supplies item-placement operations. Accepting commission materials coordinates those actual owners with story progress through an ordinary focused function, not an inventory transaction subsystem.
- ui/inventory and ui/dialogue submit intents and display results. They do not own pending products.
- save serializes selected canonical values from the character/NPC owners.

These are modules inside the one game package, registered with its existing Bevy plugin. The design does not need an entity for every recipe, stock row, commission, potion cooldown, or donated material.

## Finite unknowns and behavioral checks

Required content extraction is limited and concrete: item recipe rows; Antone/Ferro/Fuzzfas NSF branches; their NPC stock rules; and the string/quote records needed to identify exact offers. Confirm recipe duplicates, product placement expectations, commission repetition, and the service effects before claiming full fidelity.

Focused checks for implementation:

1. Reversed ingredients select the same recipe; unidentified inputs leave both values unchanged. A skill one point below the threshold rejects the recipe without practice or ingredient consumption; same-kind inputs still follow the original stacking fallback.
2. Unequal stacks preserve their remainder; mixed stack/nonstack inputs consume the right amount; nonstack output cases follow the agreed yield rule.
3. A stale input ID or repeated UI submit cannot consume a different item or create a duplicate product.
4. Product placement, same-kind fallback, and practice-before-creation RNG order match the original baseline fixtures; any deliberately chosen departure is recorded explicitly.
5. Stack merging and charge refilling neither invoke recipe practice nor discard a remainder.
6. Alchemist eligibility, profession bands, rest gate, 360-aging-tick cooldown, and overflow behavior are checked independently of manual Alchemy recipes; one delivered aging tick reduces the cooldown by one outside the camping gate.
7. Partial material acceptance, rejected offers, and closing dialogue retain the correct owner and receipt counts.
8. Scheduling the same completed commission twice creates one pending output; save/load does not replay its fact handler.
9. Stock is unavailable before and at the source's exact readiness timestamp, and matures once after it; travel does not reset the deadline.
10. Merchant maintenance cannot decay an unfinished commission or charge for stock that is not ready.
11. A finished product purchase with insufficient gold, stale stock ID, or no valid destination leaves both gold and stock intact.
12. Cancelling a service quote does not charge; accepting it applies the agreed payment/effect once, even if dialogue presentation is skipped.

These are planned behavior checks. This documentation change adds no gameplay code or test suite.

## Sources

- [D1 — original MergeItems](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L1321-L1444) and [recipe fields](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/include/wiz8/layouts/item_tables.h#L172-L181).
- [M1 — owned modern combine operation and stack/refill paths](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/inventory.cpp#L958-L1089).
- [D2 — original alchemist recipe table and brew](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/character_skills.cpp#L349-L405), [two-minute aging units](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L97-L109), and [time gate](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L883-L896).
- [M2 — modern alchemist brew](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/character_skills.cpp#L142-L199), [time gate](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L907-L920), and [camping gate activation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L115-L141).
- [D3 — Antone scheduling facts](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting%20Facts.cpp#L108-L137) and [Ferro scheduling facts](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting%20Facts.cpp#L573-L590).
- [D4 — original delayed stock](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L514-L533), [maturation](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L934-L1000), and [stock maintenance](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/npc_items.cpp#L173-L232).
- [M3 — modern stock ownership](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/layouts/npc_state.h#L16-L44), [delay](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L412-L428), [maturation/decay](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L680-L801), and [maintenance](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L157-L216).
- [D5 — original price acceptance](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/NPCInteractionSubscreen.cpp#L3458-L3489) and [price construction](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/NPCInteractionSubscreen.cpp#L4276-L4301).
- [M4 — modern price acceptance/cancellation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/npc_dialogue.cpp#L400-L456).
- [M5 — modern stale-selection validation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L444-L487) and [purchase commit](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/npc_items.cpp#L541-L584).
- [D7 — fact assignment invokes side effects independently of value change](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting%20Facts.cpp#L1026-L1050).
- [D8 — Antone ingredient and commission facts](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/include/wiz8/fact_state.h#L14-L40) and [Ferro ingredient and commission facts](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/include/wiz8/fact_state.h#L471-L491).
- [M7 — the screen's service buttons include player spells](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/NPCInteractionSubscreen.cpp#L2628-L2651).

