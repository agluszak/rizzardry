# In-game character review

The review screen is sourced from Wizardry 8 Decomp
[`075cbd0`](https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819)
(`RCSCommon.cpp`, `RCSStatsPage.cpp`, `ReviewCharacterScreen.cpp`, and
`character_skills.cpp`) and Whizzardry 8
[`31a3dfd`](https://github.com/agluszak/whizzardry-8/tree/31a3dfd57ee99fec8ff4b47f4b9be449c42de2fa)
(the corresponding screen files and `character_tables.cpp`). Frame dimensions
and artwork were checked against the extracted retail installation.

The screen has a shared portrait header and equipment, skills, statistics, and
spells pages. The implemented pages are state-scoped retained Bevy UI: the root
owns the selected character, the native tab list owns page selection, and the
statistics panel owns its filter and combat/armor selection. Native sliders own
scroll positions. Widgets read live character, vitals and inventory; they do not
retain copied gameplay snapshots. Modal focus is initialized on opening and
returns to the selected page tab when its information dialog closes. Page
visibility also controls inherited visibility, so tab navigation skips hidden
controls. Changing pages does not transfer the cursor-held item.

The item page also shows experience, health, stamina, carrying load, armor and
attributes in the retail panel, with the retail realm icons and right-click
attribute and secondary-stat information dialogs. Load includes the party pool
share. The item pool scrolls in rows using the retail arrows, thumb and wheel.
The experience bar is available at level one; the previous threshold for
characters who change profession is not yet stored.

The shared header uses the retail corner, portrait and selection rim, condition
and empty sockets, hand graphics, column art, and health/stamina strips. Text
comes from `stringdata.dat` and the retail bitmap fonts. Hand items and armor
come from the current inventory and combat rules. The level-title band is derived
from current profession levels; retail also has a separate band base for imported
characters, which is not modelled.

The bottom bar uses `ReviewPageButtons.sti` at x=420, 464, 508, and 552, with
the retail normal, hover and selected frames. The magic page uses
`ReviewMagicPage.sti`, realm icons and the retail resistance bars. It currently
shows empty spell lists because learned spells and spell points are not modelled.
Resistance includes racial, profession, attribute, skill and worn-item modifiers;
ongoing spells and other unmodelled modifiers are not included. The skills screen uses `ReviewSkillsPage.sti`
frame 2 at (0, 165), and frame 0 or 1 at (310, 0) depending on expert skills.
The category origins are weapons (352, 173), miscellaneous (352, 369), combat
(34, 174), magic (34, 299), and expert (352, 33). Category icons use
`ReviewSkillsIcons.sti` frames 2, 3, 0, 1, and 4, offset (-22, -3). Active or
nonzero skills get 273×14 rows from frame 3, with text from string IDs starting
at `0x2e2` and base/bonus segments from `ReviewSliderbar.sti`. Availability is
derived from profession, profession levels and the seven mastery thresholds.
Realm skills require learned spells, which are not implemented. Right-clicking a
skill name opens the retail skill-information popup, with its description,
controlling attributes and applicable best, closed and primary-skill notes.
The popup is modal, scrollable and owned by the review screen.

The statistics screen uses `ReviewStatsPage.sti` frame 1 at (0, 165), seven
attribute rows at 14-pixel intervals and 125-pixel retail value bars. The upper
combat/armor switch uses `commod_back.sti` and `commod_buttons.sti`; the current
combat rules supply primary-hand damage, attack and swing counts, hit and damage
bonuses and initiative. The current personal-effects area uses
`ReviewFXFilters.sti` and `main_scroll.sti`; its filters, arrows, wheel and
thumb operate on the current character's conditions and the item modifiers
currently represented in the item catalog. The screen uses the same armor-average
calculation as the main HUD.

The following retail content is still missing or incomplete:

- Learned spell/realm state, spell-list interactions, and level-up editing.
- Trait and enchantment lists, most item modifiers, kill/death counts, and the
  full armor-component and secondary-hand calculations. Unavailable numbers are
  left blank instead of being fabricated.
- The improved-skill palette (the improvement flag is not yet modelled),
  and some portrait interactions and status details.
- The game currently models neither imported-character level-band bases nor all
  skill eligibility restrictions (such as traits that prevent learning).

A software-Vulkan integration harness rendered the actual inventory scene and
loaded the retail assets, including a character portrait; the library tests
exercise live character/condition updates, filter behavior and skill eligibility.

## Comparison with original-game images

The rendered inventory and statistics pages were compared with published
screenshots of the original game: [inventory](https://www.pcgames.de/screenshots/1280x1024/2002/01/Inventory.jpg),
[statistics](https://blog-imgs-44-origin.fc2.com/w/i/z/wizardry8/RPC000039.jpg),
and [skills](https://blog-imgs-44-origin.fc2.com/w/i/z/wizardry8/RPC000051.jpg).
These references show the same page frames, portrait/header placement, inventory
cell positions, and statistics and skills panel layout. They also expose missing
behavior: the original item-action and filter controls are usable, equipment has
additional overlays, the item page can switch its information panel, and the
statistics page has more combat values and traits than the current game models.
The currently unsupported item-action buttons use their disabled retail frames.

The supplied `Wiz8.exe` was also run directly in a QEMU system guest with Wine
and its software renderer. A newly created level-one male human fighter was
reviewed from the party-assembly screen. The original's magic page shows 25 in
all six resistance fields, as does the implementation for a human fighter with
no resistance modifiers; the resistance bars, realm icons, empty spell panels and
scroll arrows occupy the same places. This is a comparison of an empty spell
list, not evidence for learned spells or casting.

The original fighter's statistics page lists four abilities and traits, whereas
the implementation currently shows only its represented skill bonus. Its item
page also shows starting equipment and armor-value ovals; the implementation
fixture had no equipment, and the implementation does not yet draw the ovals.
The original's party-assembly review
is a pre-game state: its pool and party portraits differ from the in-game
implementation fixture, and its skills do not use the in-game best-skill
highlight. Those differences must not be treated as layout regressions.

The original reached its main menu, character creator, party assembly and all
four review pages. It crashed when proceeding into the 3D world in this guest,
so in-game behavior and a comparison with identical saved character and equipment
state remain unverified. The software renderer also exposes partially redrawn
frames during capture; the side-by-side item and magic images use complete frames.
