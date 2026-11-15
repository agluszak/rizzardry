# Wandering encounters

The source for the numbers below is `MonGen.cpp` in the Wizardry 8 decomp:
`CanGenerateEncounter`, `UpdateRandomEncounterBudget`, `CullExpiredEncounters`,
`SelectEncounterCandidates` and `RollEncounterGroupSize`.

A level has an encounter budget. Its capacity is that budget clamped to the
level's minimum and maximum number of encounters. Defeating an encounter in
combat spends one budget point, within the level's budget bounds. Defeating
one outside combat frees its capacity without spending budget. Surviving
escort groups remain in the world.

Each generator has its own timer, interval, chance, encounter table and place.
Intervals vary by up to 20 percent. A normal attempt requires the place to be
70–400 metres from the party, hidden from it, and unoccupied within a 10 metre
box. Combat, conversation, the level's hold and a disabled generator prevent
generation. A due timer rearms even when its attempt is refused.

At capacity, encounters older than the level's lingering value times twelve
game seconds may retire if their leader is more than 100 metres away. Retiring
an encounter removes its main group, escorts and members; it does not spend
budget. Retirement takes effect before the attempt checks occupancy. Each
spawn takes effect before the next generator checks capacity or occupancy.

The first arrival sets the budget to its maximum. Returning grows the budget
by one per configured period spent away. More than ten game hours away
replaces existing encounters with fresh attempts; those attempts ignore
distance, visibility, occupancy and chance, but still respect capacity and
the generation gates. Shorter absences preserve encounter age. Generator
timers do not advance while the level is dormant.

Tables select by rarity, challenge relative to the party's rounded average
level, and time of day. Night is at or before 05:00, and after 22:00.
Group size uses the species' dice, difficulty and relative level. Companion
groups use their own dice and chances. Harassment's possession gate remains
unsupported, so those generators do not spawn.

Generators are level-owned entities with Bevy timers and transforms. Authored
settings become components at creation. A main group's `Encounter` component
is its active registration; removing it releases capacity. Capacity and
occupancy are queried, never mirrored. Setting a level aside retains these
entities and their relationships. There is no ordering contract for random
draws or for competing generators.
