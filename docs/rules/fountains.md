# Fountains

A fountain supplies stamina, health, spell points, or restoration. Each use
consumes one charge unless its supply is endless. A finite supply starts its
refill deadline on the first consumed charge; when that deadline passes it
returns to the authored capacity. Empty supplies refuse the action until then.

Restoration casts Heal All at the authored power, then removes conditions other
than infatuation. Power zero skips the spell; values above seven use power one.
The source is `Trigger.cpp`, `Trigger::RunAction` actions 0x25–0x2b,
`PointCastSpell`, and `RemoveAllConditionsFromParty`.

Drinking reaches every living member as a spell does: health, stamina and
spell points as the Heal, Refresh and Replenish effects; restoration as Heal
All's effects followed by Cleanse.

## How this is built

A fountain's device carries its `Supply`; drinking is a deed aimed at the
living members (`world/fountains.rs`).
