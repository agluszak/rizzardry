# In-game character review

Compared against both fixed revisions:

- [Decomp `075cbd0`](https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819),
  `src/wiz8/local_screens/ReviewCharacterScreen.cpp` (`DrawCampScreen`, page dispatch)
  and `RCSStatsPage.cpp` (`DrawCampSkillsPage`).
- [Whizzardry `31a3dfd`](https://github.com/agluszak/whizzardry-8/tree/31a3dfd57ee99fec8ff4b47f4b9be449c42de2fa),
  the same screen files (`W8CampSkillsPage::DrawCampSkillsPage`) and
  `src/data/character_tables.cpp` (skill categories).

Both references share the reviewed character across equipment, statistics and
skills. Skills distinguish base points from effective levels and group entries
into weapons, miscellaneous, combat, magic and expert skills. Retail filters
entries by active status or nonzero points/levels and can highlight the party's
best member. Review and level-up editing are separate flows.

The Rust port adds Statistics and Skills alongside Equipment under the existing
`Screen::Inventory` lifetime. `InventoryScreen` owns only the selected entity,
page and pool scroll. All page nodes remain mounted; `Node.display` and inherited visibility select the
visible page and exclude its hidden controls from native tab navigation. Shared portraits and exit controls remain outside the pages.
Bevy 0.20 headless buttons supply pointer/Enter/Space activation, with native tab
navigation and a modal focus group. The creation pages and unavailable skill rows receive the same visibility fix.
Equipment activation continues to call the
existing validated inventory operation. Changing pages neither transfers the
held item nor changes the selected actor.

Statistics reads the existing character/vitals APIs for attributes, level,
experience, HP, stamina and conditions. Skills shows base/effective values using
`Character::skill`, including its profession bonus. Retained text responds to
changes in those components or the selected member; there is no display model
copy, periodic reconstruction or separate gameplay state.

Limits: the runtime has no active-skill flag, so all 41 skills are listed and zero
is explicitly not presented as proof of training. Best-in-party highlighting,
stat information dialogs, profession history, spell points, resistances, attack
summaries and level-up editing remain unported. This uses native text and a new
layout, not a reconstruction of the retail statistics artwork. Equipment and
the shared header still need retail artwork.

## Validation and open rendering check

The focused headless review test exercises displayed HP after damage and healing,
condition expiry, and selecting another actor. It does not assert widget counts,
entity identities, or the internal page representation. Existing inventory exit
contracts remain in the library suite.

A temporary software-Vulkan harness rendered the real Statistics and Skills
builders and activated their page buttons. It exposed and helped correct a
wrapped value-column layout error. It also intermittently omitted the standalone
shared header and Equipment button, despite valid computed geometry and inherited
visibility. Disabling pipelined rendering and pacing updates did not conclusively
resolve that omission. The cause is not established; this is an outstanding visual
validation issue, not a passing end-to-end rendering check. No forced redraw or
renderer-cache workaround has been added. Full review with retail artwork and
interactive pointer/keyboard input still needs checking before merge.
