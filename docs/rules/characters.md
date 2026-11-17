# Making a character and gaining a level

A character is made once, at the start of a game, and gains levels as they
earn experience. Both are the same thing in the original: a sheet of points
to spend over what the character is, with limits on each attribute and
skill, some raises the rules force for free, and spells to pick
(`CharGeneration.cpp`, `InitializeCharacterCreation` and
`InitializeCharacterLevelUp`, both through `InitializeCharacterPointPools`).
Retail paths are under `wizardry-8-decomp/src/wiz8/local_code/` unless named.

## 1. Making a character

1. **Race, gender, profession.** A new character starts at their race's
   attributes; one below the profession's minimum is raised to it, and
   those raises are paid from the attribute pool of 60
   (`ApplyRaceProfessionTables` CG:993-1061, `ApplyProfessionMinimumAttributes`
   CG:316-366). A profession is offered only if the pool covers its
   minimums and each attribute can reach its own within its limit
   (`DetermineEligibleProfessions` CG:424-460). A Valkyrie is a woman;
   choosing Valkyrie makes the draft female, and another profession turns it
   back (CG:923-931). Changing race, gender or profession refunds what was
   spent and gives the default portrait, personality and voice
   (`DeriveCharacterPersonality` GameplayCode.cpp:942-976).
2. **Attributes.** The whole pool is spent. No attribute takes more than its
   step, a third of the pool rounded up (at least 3, at most the pool), and
   none goes above 100 (`RecomputeAttributeLimits` CG:229-274).
3. **Skills.** Each skill's base is the average of its two attributes. The
   profession's skills and bonus skill start at base × 60 / (1 + the number
   of profession skills) / 100 (`RebuildSkillAllocations` CG:717-756),
   seeded again from the final attributes. 15 skill points are spent, at most
   5 a skill, none above 75, only in skills the character may use: their
   profession's, the bonus skill, the open skills, an expert skill once its
   attribute is 100, a realm skill once a spell of the realm is picked
   (`RecomputeSkillLimits` CG:514-541, `IsCharacterSkillAvailable`). A skill
   that stops being usable gives its points back (`RefreshCharacterSkillAvailability`).
4. **Spells.** A profession of caster level 1 or more picks 2 first-level
   spells of its books (`FinalizeSpellPointPool` CG:594-635); what is
   learnable is judged without the draft's own realm and book points
   (`CountRemainingSpellPoints` CG:640-711). Picks not spent are kept, by
   book (general for a Bishop). Set Portal brings Return to Portal for one
   pick (CG:815-859).
5. **Experience.** The first goal is the profession's weight (`CalcXPGoal`).

## 2. Gaining a level

1. **Who.** A member alive, with nothing worse than Unconscious, whose
   experience has reached the goal (`IsCharacterReadyToAdvance`
   GameplayCode.cpp:307-321), outside a fight, a conversation and travel.
2. **The level.** One more level in all, and one more in the profession
   followed, which may change. The next goal is the one passed plus the
   profession's weight × 2^(level − 2), × 1.2 a level past 10 (`CalcXPGoal`).
3. **Points.** 6 attribute points, at most 3 an attribute; 9 skill points,
   at most 3 a skill; the limits of making a character otherwise.
4. **A new profession** raises the attributes below its minimums, paid from
   the 6 and counted in the cap of 3, and the new profession's skills and
   bonus skill to 5 for free; a profession whose caster level reaches 1
   raises its spellbook skills to 5 for free (CG:911-971, `ResetSkillContribution`).
   It is offered only if its deficits fit; one's own is always offered
   (CG:432).
5. **An expert skill closes** when its attribute is lowered below 100, and
   what was put into it comes back (CG:507-508).
6. **Spells.** A level at caster level 1 or more grants one pick, besides
   the picks saved (`FinalizeSpellPointPool`, `CanCharacterLearnSpell`
   Magic.cpp:1042-1093).
7. **Accepting** writes the level, the profession, the attributes, the
   skills, the picks and the spells; health, stamina, points and armour
   follow, with no healing (CG:164-169). A Gadgeteer's omnigun follows the
   level.

## How this is built

**A draft is an actor** (`game/characters/draft.rs`): what is being made,
or what a member is to become, is an entity with the sources a member has
(`Identity`, `Birth`, `Calling`, `Abilities`, `Training`, `LearnedSpells`,
`SpellPicks`, `Experience`, `Vitals`) and the race's `Nature` and armour,
marked `Drafted`. Its figures (attributes, skills, the skills it may use,
most health and stamina, resistances, spell points) are written by the same
systems as any actor's, and the screens read them: on paper, wearing
nothing and with nothing laid on it. Nothing of play takes it in: time,
fitness and falling ask for a body in play (`Placed`: a seated member or
an actor of a level), the rules of a fight for a member or a monster.

**What may be spent is the draft's**: `Points` (the pool granted: 60 less
what the profession's minimums cost and 15 to make one, 6 and 9 a level),
`Forced` (a new profession's minimums raised out of the points), `Allotted`
(what the player put in) and `Picked` (the spells picked, in order).
What is left, the step and the limits are plain rules over them
(`attributes_left`, `step`, `may_raise_attribute`, `may_raise_skill`,
`unfinished`, `unready`). Spending is an event, `Allot`, heard by one
observer for both; `Pick` by the observer of the kind of draft. A skill a
draft may no longer use gives back what was put into it (`give_back`,
following the `UsableSkills` figure); a new character's skills are seeded
again from their attributes (`reseed`).

**Making a character**: a `Newborn` draft (whose hook sets its `Conceive`
and `Pick` observers) is given a race, gender and profession by `Conceive`,
named and given a look by `Introduce`, and joins the party by `Join`
(its points kept, its picks left, its starting gear). Drafts are `MadeIn`
their creation session and go with it, but for those chosen.

**Gaining a level**: `Advance` makes a draft of the member (a copy of their
sources) that `Becomes` them, with its `Follow`, `Pick` and `Accept`
observers. `Follow` lays it out from the member for a profession: one's
own pays an attribute debt down out of the points, a point at a time, the
largest first (`PayDownAttributeDebt`); a new one raises its minimums
(`Forced`). `Accept` writes it back onto the member and the draft goes; a
draft whose member changed after it was made goes too.

`CharacterDraft`, `Advancement`, `Evaluation`, `Progression`, the
creator's `DraftActor`, `Character`, `Paper`, `Trained` and `Schooling`
are gone: no rule runs on a character that is not an entity. A character made by the original's recipe at level 1
(the companions, the tests) is `characters::born`.

What training adds to an actor's figures is a figure of its own on every
actor, nothing for one who never trained: `TrainedInitiative`,
`TrainedResistance`, `TrainedReduction`, `TrainedMaxima`, `CasterLevels`,
`UsableSkills`. No figure asks whether its actor trained.

## Open

- Retail carries an attribute debt across levels when attributes fall below
  the profession's minimums (`PayDownAttributeDebt` CG:371-417), which
  shifts the rank title; neither is here.
- Rank titles in the character screen's header (`CalcCharacterLevelBand`).
- Making a character mid-game, with gear priced against the party's gold.
- When all picks are spent, retail replaces the first picked row; the game
  replaces the lowest spell id but the last picked at creation, and refuses
  at level-up (C7). Not changed.
- An attribute debt left unpaid after a level's points (the deficit past
  6) is not carried, and shifts no rank title.
