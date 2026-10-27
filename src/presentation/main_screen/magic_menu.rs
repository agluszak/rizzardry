//! Choosing a spell or an item to use, its power and its target, for the
//! selected member: opened by the bank's Cast Spell and Use Item buttons.
//! It replaces retail's spell and item views (`MGSSpellCasting.cpp`,
//! `MGSUseItemSelect.cpp`) with plain controls until those are ported. The
//! menu holds only the unconfirmed choice; confirming sends a
//! [`MagicRequest`], which gameplay accepts or refuses.

use bevy::input_focus::{InputFocus, tab_navigation::TabIndex};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button as UiButton};

use crate::content::items::ItemCatalog;
use crate::content::spells::{SpellCatalog, SpellDefinition, Targeting, Usable};
use crate::game::characters::{Character, Vitals};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use crate::game::inventory::Carried;
use crate::game::magic::casting::{Aim, MagicChoice, Source, usable_items};
use crate::game::magic::{LearnedSpells, MagicRequest, SpellPoints};
use crate::game::party::roster::PartyRoster;
use crate::game::world::population::Monster;
use crate::presentation::screen_art::absolute;

const PANEL: Color = Color::srgb(0.08, 0.07, 0.05);
const GOLD: Color = Color::srgb(0.72, 0.58, 0.28);
const PAPER: Color = Color::srgb(0.9, 0.85, 0.72);

/// The open menu and its unconfirmed choice.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct MagicMenu {
    pub caster: Entity,
    /// Items rather than spells.
    pub items: bool,
    source: Option<Source>,
}

impl MagicMenu {
    pub fn open(caster: Entity, items: bool) -> impl Bundle {
        (
            MagicMenu { caster, items, source: None },
            absolute(Vec2::new(150.0, 210.0), Vec2::new(340.0, 235.0)),
            BackgroundColor(PANEL),
            BorderColor::all(GOLD),
            GlobalZIndex(30),
            DespawnOnExit(crate::game::screen::Screen::World),
        )
    }
}

#[derive(Component, Clone, Copy, Debug)]
enum Choice {
    Source(Source),
    Power(u8),
    Target(Aim),
    Cancel,
}

/// The powers a spell may be cast at now (retail `levels = SP / cost`, at most
/// seven); an item casts at its own.
fn powers(spell: &SpellDefinition, points: Option<&SpellPoints>) -> u8 {
    points.map_or(0, |p| p.affordable_power(spell))
}

fn usable_now(spell: &SpellDefinition, fighting: bool) -> bool {
    match spell.usable {
        Usable::InCombat => fighting,
        Usable::OutOfCombat | Usable::Camped => !fighting,
        _ => true,
    }
}

/// Lay the menu out again whenever its choice changes.
#[allow(clippy::too_many_arguments)]
pub fn draw_magic_menu(
    mut commands: Commands,
    menus: Query<(Entity, Ref<MagicMenu>, Option<&Children>)>,
    roster: Res<PartyRoster>,
    members: Query<(&Character, &Vitals, Option<&Carried>)>,
    casters: Query<(&LearnedSpells, Option<&SpellPoints>)>,
    battle: Option<Res<Battle>>,
    monsters: Query<&Monster>,
    names: super::super::names::ActorNames,
    data: Option<Res<GameData>>,
    (spells, items): (Res<Assets<SpellCatalog>>, Res<Assets<ItemCatalog>>),
    mut focus: ResMut<InputFocus>,
) {
    let Ok((root, menu, children)) = menus.single() else { return };
    if !menu.is_changed() {
        return;
    }
    if let Some(children) = children {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }
    let (Some(spells), Some(items)) = (
        data.as_ref().and_then(|d| spells.get(&d.spells)),
        data.as_ref().and_then(|d| items.get(&d.items)),
    ) else {
        return;
    };
    let fighting = battle.is_some();
    let Ok((character, _, carried)) = members.get(menu.caster) else { return };
    let mut rows: Vec<(String, Option<Choice>, bool)> = Vec::new();
    rows.push((
        format!("{} - {}", character.name, if menu.items { "Use Item" } else { "Cast Spell" }),
        None,
        false,
    ));
    // The sources: learned spells that can be cast now, or usable items.
    let (learned, points) = casters.get(menu.caster).map_or((None, None), |(l, p)| (Some(l), p));
    let sources: Vec<(Source, &SpellDefinition, String)> = if menu.items {
        carried
            .map(|carried| {
                usable_items(character, carried, items)
                    .into_iter()
                    .filter_map(|(id, spell)| {
                        let definition = spells.get(spell)?;
                        let item = carried
                            .equipment
                            .values()
                            .chain(carried.backpack.iter())
                            .flatten()
                            .find(|i| i.id == id)?;
                        let name = items.get(item.item)?;
                        Some((Source::Item(id), definition, item.display_name(name, true)))
                    })
                    .filter(|(_, spell, _)| usable_now(spell, fighting))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        learned
            .map(|learned| {
                learned
                    .0
                    .iter()
                    .filter_map(|&id| spells.get(id))
                    .filter(|s| crate::game::magic::modelled(s.id) && usable_now(s, fighting))
                    .map(|s| {
                        let power = powers(s, points);
                        let source = Source::Spell { spell: s.id, power };
                        (source, s, format!("{} ({} SP)", s.name, s.cost))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if sources.is_empty() {
        rows.push(("Nothing usable now.".into(), None, false));
    }
    let chosen = menu.source.and_then(|source| {
        sources.iter().find(|(s, ..)| same_source(*s, source)).map(|&(_, spell, _)| (source, spell))
    });
    for (source, spell, label) in &sources {
        let affordable = match source {
            Source::Spell { power, .. } => *power > 0,
            Source::Item(_) => true,
        };
        let selected = chosen.is_some_and(|(c, _)| same_source(c, *source));
        let label = if selected { format!("> {label}") } else { label.clone() };
        let _ = spell;
        rows.push((label, affordable.then_some(Choice::Source(*source)), selected));
    }
    if let Some((source, spell)) = chosen {
        if let Source::Spell { power, .. } = source {
            let most = powers(spell, points);
            rows.push((format!("Power {power} of {most}"), None, false));
            for p in 1..=most {
                rows.push((format!("Power {p}"), Some(Choice::Power(p)), p == power));
            }
        }
        let forced_self =
            matches!(source, Source::Item(_)) && fighting && spell.targeting == Targeting::Ally;
        match spell.targeting {
            Targeting::Ally if !forced_self => {
                for &member in &roster.0 {
                    let Ok((profile, vitals, _)) = members.get(member) else { continue };
                    if !vitals.is_dead() {
                        let label = format!("On {}", profile.name);
                        rows.push((label, Some(Choice::Target(Aim::Member(member))), false));
                    }
                }
            }
            Targeting::Enemy | Targeting::EnemyGroup => {
                for monster in battle.iter().flat_map(|b| b.monster_entities()) {
                    if monsters.get(monster).is_ok_and(|m| !m.is_dead()) {
                        let name = names.name(monster).unwrap_or_default();
                        let label = format!("At {name}");
                        rows.push((label, Some(Choice::Target(Aim::Monster(monster))), false));
                    }
                }
            }
            _ => {
                let label = if fighting { "Confirm" } else { "Go" }.to_owned();
                rows.push((label, Some(Choice::Target(Aim::Member(menu.caster))), false));
            }
        }
    }
    rows.push(("Cancel".into(), Some(Choice::Cancel), false));
    let mut first = None;
    for (i, (label, choice, selected)) in rows.into_iter().enumerate() {
        let column = (i / 9) as f32;
        let at = Vec2::new(6.0 + column * 166.0, 6.0 + (i % 9) as f32 * 25.0);
        let mut row = commands.spawn((
            absolute(at, Vec2::new(162.0, 22.0)),
            BackgroundColor(if selected { GOLD.with_alpha(0.3) } else { Color::NONE }),
            ChildOf(root),
        ));
        if let Some(choice) = choice {
            row.insert((UiButton, TabIndex(0), choice, BorderColor::all(GOLD.with_alpha(0.5))))
                .observe(crate::presentation::retail_controls::primary_press)
                .observe(choose);
            first.get_or_insert(row.id());
        }
        let row = row.id();
        commands.spawn((
            Text::new(label),
            TextFont { font_size: 11.0.into(), ..default() },
            TextColor(PAPER),
            Pickable::IGNORE,
            ChildOf(row),
        ));
    }
    if let Some(first) = first {
        focus.set(first, bevy::input_focus::FocusCause::Navigated);
    }
}

fn same_source(a: Source, b: Source) -> bool {
    match (a, b) {
        (Source::Spell { spell: x, .. }, Source::Spell { spell: y, .. }) => x == y,
        (Source::Item(x), Source::Item(y)) => x == y,
        _ => false,
    }
}

fn choose(
    click: On<Activate>,
    choices: Query<&Choice>,
    mut menus: Query<(Entity, &mut MagicMenu)>,
    mut requests: MessageWriter<MagicRequest>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    let (Ok(choice), Ok((root, mut menu))) = (choices.get(click.entity), menus.single_mut()) else {
        return;
    };
    match *choice {
        Choice::Source(source) => menu.source = Some(source),
        Choice::Power(power) => {
            if let Some(Source::Spell { power: chosen, .. }) = menu.source.as_mut() {
                *chosen = power;
            }
        }
        Choice::Target(aim) => {
            if let Some(source) = menu.source {
                requests.write(MagicRequest {
                    caster: menu.caster,
                    choice: MagicChoice { source, aim },
                });
            }
            commands.entity(root).despawn();
            focus.clear();
        }
        Choice::Cancel => {
            commands.entity(root).despawn();
            focus.clear();
        }
    }
}
