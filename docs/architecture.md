# Architecture and ownership

## Goal and scope

Build the game afresh in Rust using Bevy for ECS, rendering, assets, UI, input,
audio and scheduling. Preserve the rules, content, visual identity and meaningful
timing we choose to carry forward. The C++ class tree, global variables, memory
layouts, callback queues and helper names are not design requirements.

Start with one Cargo package and ordinary modules. Split a crate only when it has
a useful independent consumer or build boundary. A module can install a Bevy
plugin when it has systems/assets to register; a collection of pure rules does
not need an empty plugin.

The decomp establishes behavior. Whizzardry demonstrates some useful ownership
boundaries and exposes remaining coupling. For example, its current
[architecture](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/ARCHITECTURE.md)
already distinguishes a party fight from monster engagement, accepted choices
from running actions, and gameplay operations from presentation. Keep those
distinctions without importing its `Game` service locator or screen router.

## Bevy is the engine

Use Bevy's native assets, animation, cameras, materials, UI, picking, audio and
lifecycle rather than building a shadow game engine. In particular, evaluate
`AnimationClip`, morph targets, `DistanceFog`, `ExtendedMaterial`,
`ViewportNode`, headless UI widgets and image asset processing for their actual
fit; see [Bevy 0.20 integration](bevy-features.md).

**Gameplay authority is still explicit.** Bevy's default animation playback
runs in `PostUpdate`, whereas the party, moving colliders and navigation
advance on fixed schedules. Physical props must not have a second animation
clock that moves their visible mesh independently of collision or pathing.
Likewise, gameplay death, item transfers, door state, dialogue decisions and
outstanding scripted consequences are not determined by animation events, UI
widgets or Bevy's automatic world serialization.

One level root owns its local entities and presentation children. An existing
party entity survives travel. The root has one phase — loading, active or
failed — and one operation (`activate_level`) makes it playable once its
gameplay prerequisites exist: it builds the world, navigation and
population, consumes what was remembered of the level, places and resumes
the party, and only then announces `LevelReady`. Gameplay systems run only
while a level is active. Travel keeps the source active during preparation;
activation captures its latest state, cancels source-owned movement, removes
its entities and places the persistent party in one operation. The pending
destination owns its entrance/placement, rather than a separate global resource.
There is no partially frozen source or speculative dormant snapshot to repair
after loading fails. Combat or camping that starts during loading delays the
commit until it ends. Presentation requests and retains sky assets on the active
level; gameplay never requests them. Presentation also installs visibility on the ancestors of
its meshes and cameras. Billboard rotation belongs to a visual child, leaving
the authored prop-part pose under gameplay ownership. Cosmetic loading proceeds
on its own. Bevy parent/child relations and screen-scoped
cleanup should eliminate manual ownership tables, not introduce a new global
world manager.

Level assets have two independent typed loaders for the retail source:
`LevelDefinition` contains collision, triggers, placements, movement paths and
sight distance; `LevelGeometry` contains meshes, materials, lighting and ambient
sounds. Presentation owns the geometry and sky handles on live level entities.
Gameplay activation never requests textures or waits for visual content.
`app` composes gameplay asset loaders separately from presentation loaders;
headless play uses `MinimalPlugins` plus assets, transforms and states, with no
render sub-app or input plugin. Avian constructs colliders from collision data,
so its optional mesh/scene collider constructors are disabled.

Device bindings live in presentation and translate into `PartyMovement`,
`PartyLook`, `CampRequest` and `ToggleSearch`. The owning game systems validate
activity before applying them; neither a keyboard action entity nor an active
input context is a prerequisite for gameplay. Headless callers use those same
boundaries. Movement intent is a request, not a second copy of physical position.

## Three kinds of data

| Kind | Examples | Authority and lifetime |
| --- | --- | --- |
| Immutable content | Items, spells, monster templates, profession rules, dialogue, level definitions | Typed assets/catalogs; shared across sessions |
| Live state | Actors, inventories, combat, active map devices, quest knowledge | ECS components/resources belonging to the current campaign |
| Persistent records | Native save, dormant local actors, explored maps | Explicit versioned values captured at a defined transition |

There is no separate mutable `GameState` containing a second copy of all live
entities. Save records are snapshots; dormant records are authoritative only
while their objects are absent from ECS. Loading consumes/reconstructs those
records into live authority rather than keeping both writable. A killed
monster must not become a fresh authored monster on re-entry; local notes,
trigger states, loot and other map mutations belong to that map. Conversely,
temporary query caches, proximity, UI drafts and in-progress render playback
are not automatically persistent just because they share a struct with a
persistent flag.

Typed content IDs identify definitions. Persistent actor/item IDs identify
instances. Bevy `Entity` identifies a current runtime entity; it is not a save
identifier. Resolve persistent references after all target entities have been
created. A derived ID-to-entity lookup is an index, not another actor database.

## Live devices

A prop-backed device is the prop entity, not a second object linked back to it.
Duplicate authored names do not merge identities; only named links select the
last registered matching record. Activation resolves entity links and the level's
ordered positioned-device index. Walk-in detection keeps one previous party-eye
sample on the level, not an occupancy flag on every device. Teleportation rebases
that sample without walking the device population. Only activated damage zones
own a timer; their timing is not inferred from occupancy. Location variables
remain one named map on the level, moved directly to/from dormant records.
Content canonicalizes names once; execution needs no second numeric binding table.
Dial-position names are formed on use, rather than maintaining another key cache.

Trigger requests enter through Bevy's command boundary. Entity-local operations
borrow only their device, or the specific device and level they coordinate; they
cannot reach unrelated resources. Only loot and party-wide effects use bounded
native systems to borrow their required resources. There is no application
registry or service object. A plain synchronous coordinator borrows immutable
level definitions, runs the selected operation, follows links and commits in
order. Commands are exclusive by design; this favors a visible, complete
operation over parallelizing the parts of one player action.

Quiet fixed steps scan proximity and hazard timers through ordinary queries.
At the first entry activation, its remaining authored suffix stays in the
synchronous operation: an earlier link may start or reset a later hazard before
that hazard ticks. Pre-ticking every hazard would change timing and random draws.
No activation candidate means no exclusive whole-level command.

Movement consumes its level-bound request and rebases walk-in detection without
entering trigger execution. Presentation follows completed click commands and
interprets outcomes such as a fountain use or a bell ringing; it does not complete
required mutations. Dormant device contents and variables move back into live
components after destination preparation succeeds.

## Entities and components

| Entity | Proposed state | Why it is an entity |
| --- | --- | --- |
| Party body | World placement, collision/controller, camera attachment, movement/rest history | The party moves as one spatial body |
| Party/adventurer character | `AdventurerState`, `Inventory`, `ActionChoice`, persistent identity | Independent progression, conditions, decisions and lifetime; no independent world collider required while in party |
| World monster | `WorldCombatState`, active placement, perception, engagement, navigation intent, group relation | Independently moves, perceives and acts |
| Named/story NPC | Persistent identity, NPC/story state, optional adventurer state, inventory/stock and role-specific world state | Must survive recruitment, dismissal and map transitions without changing identity |
| Monster group | Leader/order/provocation/allied-group state | Group coordination has a lifetime and rules distinct from a member |
| Container/door/trigger/terminal | Interaction definition and mutable device state; placement/collider where needed | A world object that can be targeted or advances independently |
| Ground pile | Placement plus owned item values | World pickup target; contained items need not be entities |
| Pending spell/projectile/continuing field | Source, target/payload, delivery state, placement if spatial | Independent lifetime that can outlast its source action or consumed item |
| UI/render/audio node | Native Bevy presentation components | Presentation lifetime, not ownership of the represented gameplay item |

The current adventurer profile is `Character` (identity, training, progression
and portrait). `Vitals` owns health, stamina, conditions and their recovery
fractions; `MonsterKnowledge` owns recognition history. Recovery/damage can
change vitals without changing the profile. Their queries justified this split. Do not start with a component for
each attribute, each backpack slot or each condition counter.

### Party geometry

Formation is an ordered value attached to the party roster. Derive character
reach, facing and projectile origin from that formation where rules require it.
Do not attach eight independently walking capsules to the party. Do not force
every targeting rule to use per-character positions either: the recovered radius
spell path tests the shared party body before including eligible members. See
[combat items](systems/combat-items-and-magic.md).

The live formation grid holds member entities. Recruitment seats the new actor;
removal and lethal party damage clear its seat within that operation, without a
continuous repair system. Roster order remains the order for UI positions,
initial placement, round acceptance and deterministic tie breaking. Standing
`Order` components, held-item provenance and explicit UI selection refer to
actors, so roster edits cannot transfer those facts to another occupant.

Whether monsters still need to approach is derived from their live positions,
health, movement capability and attack reach by the same rule navigation uses.
Combat does not consume a flag written by navigation on a previous frame.
The party's path flight is also part of action legality: movement, mouse look,
manual camping and world interactions remain blocked until the flight ends.

Combat round progress is one enum: waiting, accepted (including pacing/content
waits), or executing a phase. There is no separate executing flag to synchronize.
The weapon-attack slice accepts an ordered entity-keyed queue at round start.
It locks the order, weapon identity, initiative and attack count. Execution
revalidates membership, current ability to act, that weapon's presence and
reach; combat profiles, fatigue and target defenses remain live. Editing orders
while a round executes affects the next round, including defense bonuses and
stamina recovery. A removed actor's work is cancelled, never reassigned.

Party movement during combat is one scheduled party action. Individual action
budgets after it remain combat rules, not eight copies of a movement controller.

Monster groups own their identification/counting knowledge on group entities.
`GroupMember` is an immutable Bevy relationship; its reverse `GroupMembers`
collection is maintained by the engine. Both groups and monsters are owned by
the level independently, so deleting a group does not delete its members.
Dormant records retain authored group IDs and knowledge, then rebind membership
to fresh group entities. Character recognition history is a separate
`MonsterKnowledge` component; recognition does not mutate profile/vitals data.

### NPC identity is not a promise of identical stats in both roles

Named/story NPC actor entities can remain alive for the campaign while their map
is unloaded. Their rendering, collision, navigation and active simulation are
absent then. A dormant map holds references to these actors, not stale copies of
their character sheets. Ordinary local actors can instead move entirely between
ECS and dormant records.

Recruitable NPCs have an important exception to simplistic state unification:
the original retains adventurer state while their world monster uses a different
combat profile. The proposal keeps `AdventurerState` and, when needed,
`WorldCombatState` on the same actor, with explicit role conversion. These are
different rule domains; they are not routinely synchronized character copies.
There is one inventory and one persistent identity. NPC stock stays separate.
Damage, condition changes and recovery resolve the actor's active profile.
Retaining an adventurer profile does not make it an additional world target or
cause a second aging pass. World-role attacks and loot do not automatically use
the retained adventurer's equipment.
The exact authority and conversion steps are specified in
[party and RPCs](systems/party-and-rpcs.md).

The `PartyRoster` is the sole authority for membership/slot order. Do not add a
second manually synchronized `in_party` boolean and independent member list.
Group membership likewise has one relation on each member; any ordered list
needed by a rule is derived in a defined order.

### Values rather than entities

Inventory items, merchant stock lots, conditions, learned spells, map notes,
quest facts, damage results and a player's allocation draft are values. Giving
an item an `ItemInstanceId` does not require giving it an ECS entity.

An item has exactly one owner: character inventory/equipment, shared inventory,
held-item state, a container/pile or an explicit offer in progress. UI buttons and
selections refer to it. They never own an extra live copy. Merchant stock is a
different quantity/template representation because the original normalizes sales.

## Resources

| Resource | Responsibility |
| --- | --- |
| Content catalogs | Resolve immutable definitions and retail-to-native ID mappings |
| `PartyRoster` | Ordered membership and formation |
| `PartyInventory` | Shared pool, purse and the physically held item |
| `CampaignState` | Story facts, journal, faction relationships and lasting discoveries |
| `DormantLevels` | State of unloaded ordinary world objects; the active `Level` root is an entity, not an overlapping resource |
| `GameClock` / `GameplayRng` | Explicit simulation time and rule randomness |
| `Battle` | Party-fight phase order, round counters and one executing action; absent outside party combat |
| `Conversation` | Speaker, script continuation, offered item and dialogue mode |
| `SceneSession` | A running authored encounter and the completion it awaits |
| `CampSession` | Current rest/recovery phase, wake reason and identified fade |
| `ActiveDeviceInteraction` | One lock/trap session and any accepted attempt awaiting resolution |
| Screen-local UI state | Target drafts, level-up allocations, trade selections, map view and text entry |
| Settings | Bindings, audio/video/accessibility preferences |

This table is a responsibility list, not a mandate to put every field into one
huge resource. World-object state belongs on the object. A module should query
or borrow the data it actually needs, rather than accept a catch-all `Game`.

UI state should be scoped to its screen/session; prefer a component on the UI
root when several instances can exist. Do not accumulate every possible dialog
in a permanent global `UiState` god object.

## Preparing and activating a level

The game must distinguish **requested/preparing**, **active/playable**, and
**failed** level states. The spawn of geometry is not proof that an action can
run. Required collision, level state, references and gameplay catalogs must
be ready before one activation operation permits world input, local AI,
trigger execution and party physics. Optional visual or audio content may
continue loading when it does not affect those rules. A failed requirement
has an explicit outcome; it must not make opening a chest or using a trigger
silently succeed without its required effects.

Travel prepares the destination without modifying its gameplay state or
destroying the source. At the transition boundary, settle/cancel outstanding
source-owned actions, capture persistent local state, remove source entities, then
construct and activate the destination exactly once. The game places the
body and runs entry rules only for that activated map. Failed preparation keeps
the source playable, except for separately committed operations such as an
accepted companion dismissal. Every pending callback or playback completion
must still identify its original level/session.

This is one ordinary transition, not a new generic lifecycle framework or
a second simultaneously mutable world. Its detailed failure and revisit
semantics are in [persistence and travel](systems/persistence-and-travel.md).

The current implementation treats failed navmesh loading as settled and can
activate without navigation. Navigation failure policy remains unresolved;
readiness tests do not establish mandatory navigation availability.

## Operations, systems and observation

An operation restores its own invariants before returning. Examples are
`move_item`, `consume_use`, `apply_damage`, `commit_level_up`, `sell_item` and
`dismiss_rpc`. A cross-domain operation is a readable sequence of ordinary calls.
Resolve stable authored links (for example trigger-to-prop names) once when
loading the map, rather than reconstructing mutable lookup tables by scanning
every entity on each interaction. Such indexes are derived references, never
another mutable copy of the world. Return actual outcomes, including partial
transfer and an unplaced remainder.
Do not impose rollback/all-or-nothing semantics on every interaction.

UI handlers can edit drafts directly. Gameplay requests are consumed by their
owning system. The owning operation validates its actual inputs at execution:
equipment restrictions, target status, required assets and the current activity
must not rely solely on disabled buttons, input contexts or a caller-provided
`can_use` flag. Previews call the same non-mutating rule assessment; only a
successful authoritative operation commits the result. Requests that may wait while that system is paused belong in an
owned pending value/queue. A universal event router, event-sourced database,
transaction engine or generic workflow executor is not required.

Examples of useful running systems:

- World input, interaction selection, perception and movement.
- `advance_combat`, `advance_projectiles`, `advance_spell_effects`.
- Dialogue and the few active level-specific sequences.
- Clock/recovery/condition advancement at explicit boundaries.
- UI synchronization, animation, audio and camera presentation.

Only order systems whose dependencies require it. Mandatory consequences within
an operation are direct calls: lethal damage performs death bookkeeping before
another action can use the actor. UI repaint and sound are observations after
that mutation.

`DamageApplied` reports the actor, actual HP loss and death result from the party
damage operation. Retained portrait splats consume that fact directly; recording
and playback are ordered so a new hit refreshes its animation before expiry.
Chest and bag transfers report through `ItemNews`, sharing the inventory rule
and its notification path.

A gameplay question owns its dialog through a native linked relationship. This
preserves a UI root without parenting it under the non-UI level. Replacing or
ending the question removes the view without a per-frame reconciliation system.
Likewise, an ambient emitter's sound child represents pending or active playback;
the presence of an `AudioSink` only indicates that playback is ready.

Bevy's [observer documentation](https://docs.rs/bevy/0.20.0/bevy/ecs/observer/struct.Observer.html)
does not promise a convenient gameplay order between independent observers of
one event. Its [message collections](https://docs.rs/bevy/0.20.0/bevy/ecs/message/struct.Messages.html)
normally retain only two updates. Use them for appropriate interaction and
notification work; they are not the only record that a suspended sequence still
owes a consequence. Account for deferred entity commands before a consumer needs
the entity they create.

### Choices, execution and presentation waits

Keep three stages distinct: UI draft, accepted choice and executing action.
Keeping those stages is not redundant state. An executing snapshot owns its
payload; changing a future choice cannot retarget a missile already in flight.

Presentation can control when a defined milestone becomes ready. Playback owns
clip/voice progress; gameplay owns the meaning of completion. Store the specific
playback/request identity or poll its state. Never finish an arbitrary current
scene because an unrelated old animation ended.

Logical effects can be applied at launch, impact, a specified animation point or
after presentation completion, depending on the actual mechanic. Avoid both
blanket rules, "all animation is cosmetic" and "damage always happens in an
animation callback." Native saves initially use stable boundaries rather than
promising arbitrary in-flight continuations.

## Time and randomness

Render time, simulation time, battle rounds, accelerated camping and UI/voice
holds are distinct. Associate each elapsed interval with the activity/policy
that produced it, and apply its required aging and recovery before leaving
that activity. An interval credited as the last battle round must not later
be interpreted as exploration because `Battle` has already been removed.
Store any deferred aging debt with its semantics, not as unlabelled clock
progress whose meaning will be guessed from a future ECS snapshot. Retain
fractional clock remainders instead of discarding them on each fixed step. A covered screen, a movement restriction and an explicit player pause
need not stop the same work. The [time document](systems/time-rest-and-conditions.md)
defines the relevant boundaries.

Gameplay RNG is explicit input to rules. Cursor hit-testing, hover selection,
tooltips, rendering and previews do not draw from it. Iterate rule targets in a
defined order; parallel ECS query
iteration must not accidentally choose random-draw order. Preserve recovered
per-rule rounding and roll sequences unless a deliberate change is documented.

Native saves will include RNG algorithm/version and state. Seeded container loot
uses a local generator rather than resetting the campaign stream. Both are
explicit native policies, not claims of retail save/RNG equivalence. See
[revisions](revisions.md) and [containers](systems/locks-traps-and-containers.md).

## Module relationships

| Module | Owns | Main dependencies |
| --- | --- | --- |
| `app` | Composition, platform/settings, startup and shutdown | All plugins at the composition root |
| `content` | Definitions and content validation | Bevy assets, serialization |
| `retail` | Decode/archive/ID conversion boundaries | `content`; no live gameplay orchestration |
| `game::characters` | Progression, active-role stats, conditions and skill practice | Content |
| `game::time` | Clock advancement, periodic delivery, pause policy and camp sequence | Character, magic, world and story operations |
| `game::party` | Roster, formation, recruitment/dismissal coordination | Characters, inventory, story/world placement |
| `game::inventory` | Item ownership, movement, equipment, consumption and knowledge | Content and character rule inputs |
| `game::magic` | Targeting, cast/effect lifetime and resolution | Characters, inventory, world queries |
| `game::combat` | Party fight, initiative and action sequencing | Magic, characters, inventory, world queries |
| `game::trade` | Pricing, stock and exchanges | Inventory, NPC state, character skills and clock |
| `game::world` | Interaction, movement, navigation, local actors/devices | Content and focused rule functions |
| `game::story` | Dialogue, quest facts and journal | Explicit gameplay operations |
| `game::world::levels` | Concrete monastery/Arnika/etc. sequences | World, story and the operations each sequence actually uses |
| `presentation` | Render/audio/camera/animation and map projection | Reads live state and services identified playback requests |
| `ui` | Native Bevy widgets, view state and player intents | Public game operations and queries |
| `save` | Capture, encode, restore and stable-reference reconstruction | Domain records; never UI internals |

These are source modules, not independent dynamically linked services. Shared
IDs, definitions and simple data types can live below both callers to prevent
cycles. In particular, `content` must not depend on `retail`; `game` must
not depend on `presentation` or UI; `presentation` must not be the
initializer of persistent campaign state. Importers may decode directly into
content types when that is simpler; create separate raw records only if file
layout and native meaning genuinely require different representations. A high-level operation coordinates two domains without making their
low-level helpers call each other recursively. `magic` is separate from `combat`
because identification, curses and continuing effects also run outside a battle.

## Boundaries to preserve while implementing

- Assets are shared; actor state is not stored in an asset.
- Group membership, targeting and scene attachment are references, not ownership
  that can accidentally despawn an actor.
- Render children may be owned by an actor; a party member is not owned by a
  monster group or dialogue widget.
- No view callback owns the only copy of a pending gameplay consequence.
- Changes to role, map, session and inventory owner are explicit operations.
- C++ retirement/cleanup tricks can disappear once their behavioral milestone
  has a direct representation.
- New abstractions must eliminate a demonstrated problem. Prefer small enums and
  ordinary functions to a framework for hypothetical future mods.

## Screen sessions and drafts

Mutually exclusive screen state uses Bevy states. Entry creates a scoped root
with its local selection, scroll/view data and input context; art can attach
later without controlling whether the session exists. An accepted NextState
already blocks world input before StateTransition; another opening request cannot
overwrite that accepted choice. Explicit inventory exit
returns the held item through the normal inventory operation before scoped
cleanup. A required inventory-art failure returns to the world through that same
exit operation. The held cursor overlay remains session-wide, including the world view.

Automap typing is a separate input context. A draft carries its source level,
placement, layer and text, and the expected old entry when editing. Enter submits
it to the gameplay commit operation. Escape, changing tools, closing or leaving
the level discard the draft. Persistent map notes never contain unfinished
editing, and delayed confirmation cannot overwrite a changed vector entry.

## Gameplay ordering and retained views

The application composes common campaign resources, item notices, game-data
loading and the screen state. Feature plugins register their own behavior.
Within Update, equipment precedes level activation, then facing input,
clock-requested perception and battle work. Gameplay reads physics Position and
Rotation when present, otherwise current authored Transform, never a propagated
render pose. ActorPose is a borrowed query, not another position cache.

World selection/activation runs in PostUpdate after camera updates, transform
propagation and visibility completion, so the ray and candidate meshes describe
the same rendered snapshot. Outcome consumers follow activation; UI text is
rasterized on its next presentation pass. Roster/authored order defines random
consumption within operations; cosmetic RNG remains independent.

Battle start and round completion invoke group recognition synchronously with
the actual battle participants and gameplay eye/facing. Clock requests carry
the originating level entity; a departed level cannot redirect them. Required
round recognition completes before a following action or battle removal takes
effect. Damage makes actors ineligible for further attacks and navigation,
including off-mesh links and a crossing already in progress. Death clears drive
outputs and queues crossing removal at the same command boundary as loot; body
geometry and vertical velocity remain intact. Defeat is settled before/after
combat execution and after environmental damage, independent of pacing or input.
Round-start intent is accepted before waiting in Battle's existing execution
state; it does not remain in a transient message buffer. Asking for travel
replaces the previous question with a fresh identity, rejecting stale answers.

Main-screen and inventory controls persist through display changes. Concrete
`Picture` and `BitmapText` components on actual UI entities describe their
rendered sources, layout anchors and text alignment. Systems change their
properties, visibility and bar geometry; font raster images remain cached.
Tooltips read current health/knowledge even at a stationary pointer. Bevy counts
portrait clicks with the retained 400 ms policy. The control only remembers
whether the previous click selected this actor: giving an item disarms review,
without maintaining a second pointer/timestamp tracker. Rebinding a slot retires
only its hit target, so a press cannot release onto a different actor; ordinary
health, selection and inventory display changes retain that target. Screen
sessions still own lifecycle; these display sources are not gameplay copies.

The fixed time operation follows physical movement and trigger settlement. Each
aging pass updates conditions and performs recognition before the next pass;
no transient request coalesces RNG-consuming work. Camp progression settles in
that same fixed operation. World activation creates walking bodies before battle,
so neither combat nor presentation repairs their coordinate convention afterward.
Mandatory navigation failures reject destination preparation.

## Navigation importer and processor caching

Retail's navigation loader converts OCT/PVL into typed `LevelCollision`.
Gameplay navigation consumes those triangles and size class; it imports no
retail sections, flags or byte cursor. Body metadata likewise loads independently
of optional monster meshes, materials and animation cycles.

The small synthetic navmesh input is retained for processor cache invalidation:
its source bytes include both retail files and the size class. Replacing this
with `LoadContext::read_asset_bytes` dependencies is not equivalent in the
current source configuration. In Bevy 0.20, processor dependency reads require
processed metadata with `ProcessedInfo.full_hash`; `retail://` deliberately
serves raw assets through both readers and has no processed metadata. A direct
substitution would fail to obtain that hash. Any later replacement must first
prove invalidation/rebaking across both source files and bake settings. This is
source-level evidence; a retail cache-rebake experiment has not been run.
