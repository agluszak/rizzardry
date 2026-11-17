# Items and inventory

Keep physical items as entities. Rewrite how inventory actions execute. Equipping
a two-handed weapon currently estimates space, queues two placements and a gift,
then searches for room again. A merchant checks capacity, queues splits and
deliveries, and debits gold. Combat queues expenditure and independently works
out whether another shot is possible. Each caller needs the actual outcome.
[Item rules](../rules/items.md) · [Current implementation](../../src/game/items.rs)

## State and identity

| Fact | Owner |
| --- | --- |
| Definition identity, stack count, remaining charges, knowledge, released curse | Item entity. Moving or selling preserves the instance. Splitting creates a second stack. |
| How it attacks, protects, fits or casts | Capability components made from content. Each gameplay reader queries the capability it uses. |
| Placement | One of worn slot, packed slot, pool, party hand, stowed contents, or ground placement with level and pose. Inventory owns transitions. Preserve the lifecycle of contents when their holder is set aside. |
| Ordering | Pool rows use the ordered relationship target. Pack holes and equipment slots have explicit slot identities. The stored weapon set stays addressable while inactive. |
| Gold | Party purse. Trade changes it when settlement succeeds. |
| Interaction | The party hand persists as gameplay state. Scroll position, selected cell and open item card belong to presentation. |

Keep existing placement relationships and use their reverse lookups. Adding
holder arrays alongside them would introduce a second authority. Reserve raw
placement for construction and the internals of validated inventory operations.

## Execution and outcomes

One queued request executes one gameplay action. Inside that action, call
placement and quantity rules directly and make their results available to the
caller immediately. A narrow inventory function may take mutable world access
to move several entities. Keep fit calculations and compatibility rules ordinary
read-only functions. Other domains retain the decisions that give the move
meaning.

| Operation | Result the caller needs |
| --- | --- |
| Take or give | Amount accepted, recipient or destinations, and remainder at the source. Follow the specified pack/pool priority and stack order. |
| Exchange with hand | The result of taking, stacking, loading, equipping or exchanging at this destination. Validate all necessary displacement before moving gear. A refused attempt may reveal a curse. |
| Swap sets | One resolution across four slots including the partner exception. Player input, turncoat behavior and combat fallback share it. |
| Load or spend | Actual quantity transferred or consumed, remaining uses and whether the item survived. Preserve empty reusable items and distinguish charges from stack count. |
| Buy, sell or craft | The owning rule completes the full exchange. Deliver exactly the paid quantity. For crafting, determine product placement and ingredient remainders before consuming either ingredient. Preserve the documented product-placement exceptions. |

**Two-handed exchange.** Read the incoming weapon, destination, other hand and
wearer. Determine which items leave. Check fit against the vacated slots and
check both departing weapons for binding curses. Select actual stack destinations
and a free slot or pool row for any displaced partner. Use that very selection
to perform the move. The previous occupant goes to the party hand. If capacity
fails, all three items stay put. If a curse blocks removal, reveal it and keep
the gear in place. Equipment readers see the completed change. A few local
destinations suffice without a general transaction or rollback system.

**Partial pickup.** If six of ten arrows fit, add six to the receiving stacks
and leave four in the original stack in the bag. Determine accepted quantity
before making a split entity. Report the amount even if a full merge consumes
the source entity. Apply the same compatibility and capacity rules to a purchase,
but decide whether the entire quoted quantity fits across all its stock items
before changing goods or gold.

## Integration

- **Combat and casting.** Retain the actual item entity supplying an attack or
  cast. At each existing rule-defined spending point, consume the item or charge
  and use the returned balance. Combat owns whether another shot proceeds.
  Magic owns when a cast outcome spends the item. [Swings](../../src/game/combat/swings/mod.rs) ·
  [Casting](../../src/game/magic/casting/deed.rs)
- **Equipment and actor figures.** Offer a cohesive query for active equipment.
  Armour and weapons read the relevant components. Refresh affected wearers
  on either side of a transfer and account for changes in place, including
  charges or an omnigun tier upgrade. Preserve the gun entity and its mutable
  state. When changed abilities make equipment unusable, apply the existing
  forced-removal rule through the same storage operation. [Armour](../../src/game/actors/armour.rs) ·
  [Omnigun](../../src/game/inventory/omnigun.rs)
- **Burden.** Party-wide load depends on stack quantities, personal and pooled
  weights, capacity, roster order and living recipients. Preserve pool sharing,
  tie order and combat timing. A generic inventory-changed notification cannot
  capture every relevant input. [Weight distribution](../../src/game/items.rs)
- **Trade.** Keep merchant stock as real item entities. Trade owns the current
  quote, funds, eligibility for sale and charge restoration. Inventory selects
  and moves the exact units. Complete delivery and payment together before
  reporting success. [Settlement](../../src/game/npcs/trade.rs)
- **Recruitment.** Recruitment determines wanted gear and bound equipment.
  Retained gear stays with the companion entity across dismissal and rejoining.
  Return the rest to the party pool and leave overflow at the party's feet.
  Preserve charges, identification and upgrades. [Dismissal](../../src/game/npcs/recruit.rs)
- **World.** World interaction checks reach, visibility and drop position.
  Inventory handles accepted quantities. Use the actual remainder when deciding
  whether a bag is empty. Preserve crucial-item drop restrictions.
  [World items](../../src/game/world/items.rs)
- **Story.** Today an Update sweep infers possession, adds or removes `Owned`,
  and story treats `Add<Owned>` as acquisition. Two transfers between sweeps
  can hide a real pickup. Emit a qualifying acquisition after the completed
  action with item kind and amount. Internal moves stay inside party possession.
  Verify purchases, crafting, initial outfitting, joins and companion returns
  against retail before deciding which trigger story effects. [Acquisition observer](../../src/game/npcs/story.rs)
- **Presentation.** A click on an empty pack cell targets a place. Identification,
  merchandise confirmation and scroll use target an item entity. Resolve pool
  rows for display and insertion; keep the selected item identity through
  confirmation. Display the result or refusal returned by gameplay.
  [Inventory UI](../../src/presentation/inventory.rs)

## Scope of the rewrite

Replace the item movement model and every caller in one coordinated change.
The implementation is complete when bag pickup, equipment exchange, weapon
sets, trade, crafting, recruitment, combat, magic and story all use the new
contracts. Delete the superseded Put, Stack, Rejoin, Lose and Spend observer
chains and the possession sweep. Replace the broad Items accessor with cohesive
equipment and belongings queries and direct capability reads.

Resolve stack knowledge compatibility against retail before implementing the
shared rule. The spec says the party knows both items alike; current code
compares identification. Preserve the distinction between attempting to stack
items of one definition and being allowed to combine them.

Discard tests built around deleted components, events or intermediate steps.
Verify the rewrite through a partial pickup with an exact remainder, a blocked
exchange that reveals a curse, a purchase that delivers exactly what was paid
for, and the last shot or charge affecting the next action.
