# Combat items and magic

## Scope and comparison

Worked example: an actor uses a stackable bomb whose definition carries a radius
spell. This is the original `UseItem -> CastItemSpell -> CastSpellFromSource`
path, not a throwing knife's physical weapon attack. The retail item database is
absent, so this document does not invent a named bomb's exact spell/power.

Original evidence:

- [Item selection and combat access](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MGSUseItemSelect.cpp#L405-L562).
- [Action execution](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat.cpp#L2125-L2343).
- [Item cast](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L3703-L3798)
  and [consumption](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L1234-L1279).
- [Cast setup/target storage](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic.cpp#L2706-L3224)
  and [effect advancement](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic.cpp#L586-L713).

Whizzardry has moved item use into
[Inventory](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/inventory.cpp)
and made [combat state](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/combat/combat.h)
more explicit. Its spell code still coordinates pending effects with gameplay
and presentation. Retain the real milestones; replace pointer provenance and
global access with owned payloads and explicit queries.

## Data

| Data | Owner |
| --- | --- |
| Spell, power, allowed users, quantity kind and item sound | Immutable `ItemDef` |
| Radius, targeting, effect behavior and delivery/application policy | Immutable `SpellDef` |
| Bomb stack | Value in actor's `Inventory` |
| Point currently under cursor | Screen-local `ItemAimDraft` |
| Confirmed future action | Actor's `ActionChoice` |
| Running action and its wait | `Battle.current_action` |
| Accepted delayed cast | Entity with `ActiveSpellEffect` |
| Travelling representation | `Projectile` data/entity associated with the effect |
| Damage/conditions | Target's active-role combat state |

```rust
struct ItemRef { owner: ItemOwner, id: StackId }
struct PlannedItemUse { item: ItemRef, target: TargetSpec }
enum TargetSpec { Actor(Entity), Area { center: Vec3 } }
struct ActiveSpellEffect {
    source: EffectSource,
    spell: SpellId,
    power: u8,
    targets: Vec<Entity>,
    phase: EffectPhase,
}
```

These are sketches. Store effect-specific payload where needed; do not retain a
borrow of the item value across consumption. An effect can need several visual
children, but an instantaneous damage result is not another entity.

## 1. Prepare and choose

| Step | Proposed responsibility | State change |
| --- | --- | --- |
| Select actor and open item use | `handle_combat_ui` | Create draft; apply the appropriate world/input hold |
| Choose bomb from that actor's equipment/backpack | `begin_item_aim` | Check basic usability and configure its embedded spell's targeting |
| Aim at a place | `update_area_target_preview` | Invert screen/world picking and evaluate range, radius and eligible candidates without RNG |
| Confirm | `choose_combat_action` | Store item identity/provenance and the world point as a chosen action |
| Cancel the draft | `cancel_item_aim` | Remove transient preview and restore the previous valid choice |
| Edit choice before its initiative | `choose_combat_action` | Replace the choice; inventory remains untouched |

The ordinary combat picker disables shared-pool access. A separate held-item
route must be designed from its own rules; it does not make every shared item
freely usable in combat. Basic eligibility includes allowed user/placement,
embedded spell and remaining uses.

The selected item remains in inventory. No hidden reservation inventory is
needed. Moving/replacing/emptying its source invalidates the affected choice;
execution checks identity again. The original invalidation helper's name
`DropCharacterFromRound` is misleading: it restores the standing action/target,
not necessarily the actor's whole initiative phase.

## 2. Begin the scheduled action

`advance_combat` freezes the accepted choice into an executing snapshot when its
initiative arrives. Condition interruption can cancel it, clear webbing while
cancelling it, or replace it with another action. Invalid actions can fall back
to defending. No consumption does not imply a free replacement turn.

Within `begin_item_use`, call ordinary functions in order:

1. Resolve actor and owned item; validate actor, item and target.
2. Determine item-defined power and fatigue. An ordinary throwing-item use uses
   its spell power, not a freely chosen player spell level.
3. Compute use difficulty from the appropriate skill and combat pace; include
   the unidentified-item penalty. The recovered throwing path has its own
   failure scaling/backfire rules, distinct from weapon accuracy.
4. Roll failure/backfire and applicable condition-based aim alteration using
   `GameplayRng`. Blindness-related point scatter is distinct from physical
   melee fumble logic.
5. For a non-fizzle, collect actual victims and create the owned cast/delivery.
6. Consume a use for an accepted attempt.
7. Apply fatigue and skill practice according to the outcome, then enter the
   action's resolution wait.

For this throwing presentation, the recovered ordinary item path sets fatigue
to ten. It does not spend spell points as if the character had cast a learned
spell. Keep this rule in item use rather than in the projectile renderer.

## 3. Consumption and target capture

| Result from item casting | Inventory consequence |
| --- | --- |
| Rejected (`0`) | No removal |
| Normal cast (`1`) | Consume one stack unit/use |
| Fizzle (`2`) | Consume one stack unit/use |
| Backfire (`3`) | Consume one stack unit/use |

Original cast setup can already create the missile before returning. Inventory
depletion then happens in the same action execution, before flight/impact
resolution. `consume_use` handles stack depletion, charge depletion and reusable
exceptions, updates derived inventory consequences and preserves no dangling
reference to the consumed value.

Radius spell victims are collected at cast initiation and stored. The geometric
checks include radius/line of sight/eligibility and spell-specific exclusions.
The party is tested through shared party geometry and then expanded into eligible
members. The preview list is not authoritative and is recomputed at execution.
See the original [radius selection](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic.cpp#L3838-L3922).

Do not replace this with a new overlap query at impact. Keep reference validity
and effect-specific eligibility checks at resolution, but do not silently add
new victims because they moved into the visual explosion.

## 4. Launch, flight, impact and application

The item-spell path launches directly; no player hand-animation release callback
was found. Request launch sound/visuals from the accepted action. A later visible
throwing-hands animation must follow the timeline, not redefine consumption.

```text
Accepted cast -> Flying -> Impact presentation -> Applied -> Finished
```

This is a phase sketch, not a universal fixed-delay script. The original:

1. Advances missile motion/collision and any missile impact cycle.
2. Reaches delivery completion.
3. Releases the missile and creates the spell's impact visual at its actual
   location, without applying targets in that same branch.
4. Applies on a later eligible update. The spell's `missile_delivered` flag,
   sustained status and visual/missile readiness decide whether to wait longer.
5. Retires the remaining visuals after their lifetime and releases the action's
   required wait.

Some effects apply while their impact visual runs; others wait. Express the
actual definition-driven readiness rule. `advance_projectiles` and
`advance_spell_effects` can be separate ordered systems. Neither a collision
observer nor an unrelated animation completion gets to apply damage independently.

## 5. Resolve the spell

`resolve_spell` selects an ordinary effect function from the definition. It
performs required provocation, magnitude rolls, resistance/mitigation, absorption,
HP changes, condition attempts, death bookkeeping and follow-up/reflection in
the recovered order. A spatial continuing field deserves persistent effect
state; a single HP subtraction does not.

Ordering is per spell, not necessarily all effects per victim. The original
[Noxious Fumes dispatch](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L3417-L3420)
does a damage pass, then nausea, then unconsciousness. Damage rolls are per
target; conditions have their own checks. Reordering passes changes RNG and
interactions with death. This is a spell example, not evidence for a named bomb.

Death changes combat eligibility immediately. Attribution, dependent-effect
release and relevant kill/XP consequences are explicit calls. Hit/death animation,
numbers and log entries present the result. Finish the action only after its
specific required effects/cues; do not wait for every particle or audio entity
in the world. Round completion owns clock/recovery/practice/engagement updates.

## Physical throwing and other spells

Physical thrown weapons use attack/swing/hand scheduling and physical hit
resolution, with their own depletion and replenishment boundary. They are not
implemented by making every thrown item an item spell. Conversely, identification,
Remove Curse and continuing magic use `game::magic` without requiring `Battle`.

## Persistence, checks and unresolved points

Initial manual saves are refused while an action is unresolved; autosave may
wait for an eligible stable boundary. Neither replays `begin_item_use` after an
item has already been spent. A later mid-action save
feature would need the owned effect phase/payload and persistent references.

Useful checks: cancel aim spends nothing; interrupted use keeps the bomb but
resolves the turn correctly; fizzle/backfire spends exactly one; last-unit
consumption leaves a working effect; moved actors do not cause fresh area
selection; a lethal hit removes action eligibility before the death animation;
preview redraws never consume RNG.

Open evidence: the recovered radius side selector is inconsistent with adjacent
all-enemies code, and one `AimAtPlace` helper appears to omit a position copy.
Verify friendly-fire/aim behavior with retail fixtures or disassembly before
treating those branches as an executable specification. Exact content-defined
delivery rules require the retail database as well.
