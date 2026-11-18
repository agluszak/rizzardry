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

### What the relationship guarantees

Bevy's relationship hooks maintain the reverse collection when the source
edge is inserted, replaced or removed. Once a move has been applied, asking a
wearer which items point to it has a structural answer. There is no inventory
settlement pass to run first. A queued command still has to execute before a
query can see its effects. [Bevy relationship implementation](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/relationship/mod.rs.html)

This gives the two-handed exchange a precise division of responsibility.
The hooks can index two items that both claim the same slot. They can also
index one item under both Worn and Packed, since those are separate relationship
types. Inventory must validate slot occupancy, enforce exclusive placement and
complete all displacements before announcing success. A hook on Worn removal
cannot announce a successful gift or re-equip a displaced weapon. At that
point the operation can still be between placements.

The current five reverse collections also specify linked_spawn. In Bevy that
makes despawning the holder despawn the related items. Decide that lifetime
for each holder. Dismissing a companion who keeps gear stowed requires
preserving the holder or moving those items before despawn. Opening a bag
and buying its contents need distinct outcomes even if both ultimately
remove the same containment edge. Save the game's meaningful slot and pool
order and restore edges through the inventory owner.

**An item address and an item identity serve different operations.** Preserve
the existing Place value for addressing a destination. Use Entity for a
specific item selected by the player, even when a pool row later changes.
Today Items::bearer calls place, which can search an entire pool for the row
of an item just to return no bearer. Make bearer follow Worn or Packed
directly. Reserve row lookup for a caller asking for a row. The pool already
uses Bevy's insert_related. For a Vec target Bevy appends new edges, ordinary
edge removal preserves survivor order, and insert_related can reposition
an existing member with place. Thus an ordered pool needs no parallel row
components or renumbering pass. A pack slot, in contrast, is an address that
can remain empty when another item moves. [Item readers](../../src/game/items.rs) ·
[Bevy ordered insertion](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/relationship/related_methods.rs.html) ·
[Collection implementation](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/relationship/relationship_source_collection.rs.html)

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
| Buy, sell or craft | The owning rule completes the full exchange. For a normal purchase, try pool storage and drop the overflow through the world item rule; account for every purchased unit. For crafting, determine product placement and ingredient remainders before consuming either ingredient. Preserve the documented product-placement exceptions. |

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
the source entity. For a purchase, fill compatible pool stacks, then free rows, and pass the
remainder to the existing drop-or-refuse rule. The purchase accounts for that
remainder in its quantity and price. Pool capacity alone cannot reject it.

## Integration

- **Combat and casting.** Retain the actual item entity supplying an attack or
  cast. At each existing rule-defined spending point, consume the item or charge
  and use the returned balance. Combat owns whether another shot proceeds.
  Magic owns when a cast outcome spends the item. [Swings](../../src/game/combat/swings/mod.rs) ·
  [Casting](../../src/game/magic/casting/deed.rs)
- **Equipment and actor figures.** Offer a cohesive query for active equipment.
  Combat reads the current weapon and ammunition at resolution; the turn keeps
  the weapon identity accepted earlier. For retained equipment-derived values
  such as armour, cover both wearers on transfer and in-place changes such as
  an omnigun tier upgrade. Preserve the gun entity and its mutable
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
- **Story.** Successful automatic storage in the pool or character backpack
  reports acquisition. A successful direct equip returns earlier. Crafting
  places the result directly. A dismissal can report an item already carried
  by the party. Represent the qualifying storage outcome explicitly, with the
  reported item kind available after a merge. Partial absorption followed by
  a full-pool failure changes quantities without a successful pool report.
  Deliver the acquisition facts before the exceptional-item reaction. See the
  [retail call paths](retail-evidence.md).
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

Stack compatibility requires equal item IDs and equal identification flags.
Use the destination definition's maximum quantity. A partial transfer leaves
a live remainder and a full merge consumes the source.

Discard tests built around deleted components, events or intermediate steps.
Verify the rewrite through a partial pickup with an exact remainder, a blocked
exchange that reveals a curse, a purchase that delivers exactly what was paid
for, and the last shot or charge affecting the next action.
