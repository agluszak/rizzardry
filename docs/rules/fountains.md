# Fountains

A fountain supplies stamina, health, spell points, or restoration. Each use
consumes one charge unless its supply is endless. A finite supply starts its
refill deadline on the first consumed charge; when that deadline passes it
returns to the authored capacity. Empty supplies refuse the action until then.

Restoration casts Heal All at the authored power, then removes conditions other
than infatuation. Power zero skips the spell; values above seven use power one.
The source is `Trigger.cpp`, `Trigger::RunAction` actions 0x25–0x2b,
`PointCastSpell`, and `RemoveAllConditionsFromParty`.

Drinking creates a deed aimed at the living party members. Health, stamina and
spell-point restoration use the ordinary Heal, Refresh and Replenish effects.
Restoration uses Heal All's effects followed by Cleanse. The fountain owns its
supply; the effect pipeline owns changes to actors. Deeds retain their resolved
effects and spell parameters rather than looking up effect rows as they land.
