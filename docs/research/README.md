# Retail research notes

Format and behaviour notes gathered while writing the retail decoders and
rules. Every claim cites the [Whizzardry 8](https://github.com/agluszak/whizzardry-8)
reconstruction (`path:line`) or was checked against the retail files; the
original-binary decomp was not available when they were written, so treat
behaviour drawn only from Whizzardry as reconstructed, not proven.

Where a note mentions a Python checker (`*.py`), that was a throwaway tool and
is not kept; the Rust decoders and their retail tests
(`cargo test -- --ignored`) now verify the same facts.

| Note | Covers | Implemented in |
| --- | --- | --- |
| [pvl-sections](pvl-sections.md) | `.pvl` sections after the meshes: lights, props, triggers, environment | `src/retail/sections.rs`, `trigger.rs`, `prop.rs` |
| [oct](oct.md) | `.oct` collision surfaces and their flags | `src/retail/oct.rs` |
| [environment](environment.md) | Fog and sky light by time of day, baked and dynamic lighting, sky worlds, `.ifl` | `src/retail/level_loader.rs`, `src/presentation/environment.rs`, `sky.rs` |
| [audio](audio.md) | Voice, sound and music files | `src/presentation/voice.rs`, `effects.rs` |
| [ui-assets](ui-assets.md) | STCI images, fonts, automap art | `src/retail/stci.rs`, `src/presentation/automap.rs` |
| [monster-anim](monster-anim.md) | `.mls` species and `.mon` vertex-frame cycles, playback | `src/retail/monster.rs`, `src/presentation/monsters.rs` |
| [monsters-db](monsters-db.md) | `monsters.dbs` records | not yet |
| [level-population](level-population.md) | Which monsters and NPCs stand where on arrival: `.sts` `MONS`/`MONG` layouts, encounter tables, level-entry rules, Monastery 1 example | not yet |
| [inventory](inventory.md) | Party item storage, equipment slots and usability, stacks and charges, core item moves, curses' move rule, load | not yet |
| [character-review](character-review.md) | In-game statistics and skills, compared against both references; native retained review pages | `src/presentation/inventory/review.rs` |
| [characters](characters.md) | Character record, race/profession/gender/attribute/skill ids and item-requirement id spaces, race and profession tables, XP and levels, derived stats (HP, stamina, SP, resistances, initiative, attacks, AC), a minimal valid character | not yet |
| [trigger-actions](trigger-actions.md) | Trigger actions, recipients, gates, the Monastery 2 bells | `src/game/world/triggers.rs`, `scripts.rs` |
