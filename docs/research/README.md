# Retail research

These notes record reverse-engineering observations about Wizardry 8 file
formats, UI, and rules. They are reference evidence, **not** an implementation
plan or a description of Rizzardry's current behavior. Read the Rust code for
the latter.

The notes cite [Wizardry 8 Decomp](https://github.com/agluszak/wizardry-8-decomp)
and/or [Whizzardry 8](https://github.com/agluszak/whizzardry-8).
Findings derived only from Whizzardry are reconstructed, not proven retail
behavior. Reported file counts and format checks came from retail-data surveys;
some Python checkers mentioned in the notes were throwaway scripts and are not
part of this repository.

## Files, assets, and levels

- [PVL](pvl-sections.md), [OCT](oct.md), [environment](environment.md),
  [UI images/fonts](ui-assets.md), [audio](audio.md).
- [Monster animation](monster-anim.md), [monster database](monsters-db.md),
  [level population](level-population.md), [trigger actions](trigger-actions.md).

## Gameplay and UI

- [Characters](characters.md), [inventory](inventory.md), [combat](combat.md).
- [Main screen](main-game-screen.md), [review items](review-items.md),
  [character review](character-review.md).
