# Architecture and ownership

Rewrite the implementation, preserve observable game behavior. One Cargo package
and ordinary modules are enough until an independent consumer/build boundary
justifies a split. [System designs](README.md) describe intended outcomes, not a
required class tree. [Known gaps](known-gaps.md) distinguishes unfinished work.

## Authority

- Typed assets/catalogs own shared definitions. Live ECS components/resources own
  campaign state. Save and dormant records are explicit values, authoritative
  only while their corresponding live objects are absent. No parallel GameState.
- Content IDs identify definitions; persistent IDs identify instances; Entity
  identifies the current runtime object. Restore references after creating targets.
  An ID lookup is an index, not a second actor database.
- Character owns identity/training/progression; Vitals owns health, stamina,
  conditions and recovery fractions; MonsterKnowledge owns recognition history.
  Split by independent queries and lifetimes, not by individual fields.
- The party is one entity: Formation, PartyInventory and, once it enters a level,
  its single spatial body. Members point to it with PartyMember; the ordered
  PartyRoster it maintains owns membership and order. Membership does not own a
  character's lifetime. Formation holds actor identities, not replaceable slot
  indexes; character reach is derived where the rule needs it. Not every spell
  uses individual positions.
- Relationships carry ownership: LevelOwned/LevelEntities is what a level holds
  (monsters, groups, items, props, devices, navigation, its battle) and despawns
  with it; `ChildOf` is only spatial. Combatant/Combatants is fight participation,
  ended without despawning anyone.
- Named NPC identity can survive role and map changes. Retained adventurer stats
  and world-monster stats are distinct retail rule domains, not synchronized
  copies of one profile. Make recruitment/dismissal conversion explicit.
- GroupMember/GroupMembers describe membership; both groups and monsters belong
  independently to the level. Removing a group must not despawn its members.
- Items can be ordinary owned values. A ground pile, independent projectile or
  live device warrants an entity; every backpack slot does not.
- Physics owns live Position/Rotation; use authored Transform before a body exists.
  Rendering follows that pose. Do not add another mutable position cache.

## Level lifetime

An accepted entry (a Destination with its Entry: new game or travel) owns its
requested entrance/placement and prepares mandatory collision, navigation and
population content, failing on any that is missing. Loading geometry is not
readiness. LevelDefinition is gameplay content; LevelGeometry and sky are
presentation assets. Headless gameplay installs no renderer, input bindings or
audio. Preparation draws no gameplay RNG, consumes no dormant record and moves
nobody, so a failure only discards the entry (LevelEntryFailed).

While an entry is pending, gameplay and physics are suspended and the source stays
intact (retail's please-wait load), so no fight or camp can begin mid-load. One
activation operation then captures the source's map state, retires it, restores
the destination, creates a new game's party, places the party, and announces
readiness. Unlike retail, which unloads first, a failed entry resumes the source.
With a renderer, presentation also loads the destination's appearance during
preparation and holds the commit (PresentationReadiness) until it and its
dependencies have loaded; a level that cannot be shown fails the entry. Headless
play never holds it. Retail's first-visit and save-restore entry effects (sight
state, clearing continuing spells) wait for saves and spells to exist.

Dormant records retain deaths, wounds, items, device state, variables, recognition
and notes. Restore them rather than respawning authored defaults. Do not persist
proximity caches, temporary timers, UI drafts or render playback indiscriminately.
Disk saves and dormant-time catch-up are still planned.

## Operations and ordering

Public gameplay operations validate prerequisites from authoritative state and
commit mandatory consequences coherently. Death settles before the next attack;
item transfers preserve ownership; party acceptance validates before spawning.
Input contexts and UI previews are not permission checks for gameplay.

Use messages for requests and observers for appropriate interactions/notifications.
Do not split required mutation into loosely ordered event chains. Work spanning
frames lives in explicit state: editing, accepted, executing and presentation waits
are different phases. Accepted rounds retain actor/weapon identity and order;
execution revalidates membership, capability, weapon presence and reach. Edits
apply to the next round; departed actors' work is cancelled, not reassigned.

A prop-backed device is its prop entity. Resolve authored links on activation,
retain authored execution order and coordinate linked effects synchronously.
Quiet fixed steps use ordinary queries; an activation can alter later hazards,
so pre-ticking all hazards would change timing and RNG consumption.

Clock advancement carries its originating level and activity policy. Settle aging,
recovery and recognition under that policy before discarding it. Retain fractional
time; Local caches cannot own history. Gameplay RNG uses explicit, defined actor
order. UI/cosmetics never draw from it. Native save RNG version/state and local
container RNG are deliberate policies, not retail equivalence claims; see
[revisions](revisions.md).

Physical movement and trigger settlement precede fixed-time consequences. Picking
runs after camera updates, transform propagation and visibility so ray and geometry
refer to the same snapshot. Presentation may wait for impact/dialogue completion
when gameplay requires it, but cannot own the only pending consequence. See
[Bevy integration](bevy-features.md) for visual versus physical animation clocks.

## UI and module boundaries

Session and screen states own scoped UI roots, selection and drafts. Create input
lifetime independently of delayed artwork. An accepted screen transition already
blocks world input. Inventory exit returns the held item before cleanup, including
on art-load failure. Retained controls update properties instead of rebuilding.

Native focus/widgets own interaction. Map drafts keep source level, placement,
layer and expected old entry; gameplay validates commit. Cancellation discards the
draft. Settle native edit completion after input routing to avoid a single Escape
both cancelling an editor and closing its screen. Rebinding a portrait retires its
hit target so an old press cannot release onto a different actor.

Dependency direction: retail decodes into content; game consumes content;
presentation observes game and submits intents; app composes plugins. Content
never imports retail, and rules never import UI/meshes/playback. Plugins register
integration, not an unconditional demo campaign. Pure rules need no empty plugin.

## Navigation cache

A level's navmeshes are baked when it is entered, never at startup. The
`navmesh://` input of a size class is the class plus the level's `.oct` and
`.pvl` bytes; the cache key hashes that input, the bake settings and the file
format version, so a change to any of them re-bakes. A toolchain whose hasher
changes only re-bakes too. Writing a navmesh removes that input's files under
other keys. The `navmesh_prebake` example bakes every level the same way.
