//! Read-only character review. The screen owns selection; actors own every value shown.
use super::super::screen_art::Picture;
use super::{InventoryScreen, absolute};
use crate::content::characters::{Attribute, Skill};
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Lasting, Vitals};
use bevy::input_focus::{InputFocus, tab_navigation::TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::{Activate, Button as UiButton};
use enumset::EnumSet;

const PAPER: Color = Color::srgb(0.91, 0.88, 0.80);
const GOLD: Color = Color::srgb(0.83, 0.68, 0.40);
const PANEL: Color = Color::srgb(0.075, 0.09, 0.115);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Items,
    Statistics,
    Skills,
}

#[derive(Component)]
pub(super) struct PageButton(Page);

#[derive(Component)]
pub(super) enum Field {
    Attribute(Attribute),
    Skill(Skill),
    Progress,
    Vitals,
    Conditions,
}

fn label(commands: &mut Commands, parent: Entity, at: Vec2, width: f32, value: &str) -> Entity {
    commands
        .spawn((
            Text::new(value),
            TextFont { font_size: 12.0.into(), ..default() },
            TextColor(PAPER),
            absolute(at, Vec2::new(width, 80.0)),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn panel(commands: &mut Commands, root: Entity, page: Page) -> Entity {
    commands
        .spawn((
            page,
            Node {
                display: Display::None,
                ..absolute(Vec2::new(0.0, 165.0), Vec2::new(640.0, 280.0))
            },
            BackgroundColor(PANEL),
            ChildOf(root),
        ))
        .id()
}

pub(super) fn build(commands: &mut Commands, root: Entity, sheet: Handle<SpriteSheet>) {
    // The four page buttons occupy 44-pixel cells in the retail bottom bar.
    for (i, (page, base)) in
        [(Some(Page::Items), 0), (None, 10), (Some(Page::Skills), 15), (Some(Page::Statistics), 5)]
            .into_iter()
            .enumerate()
    {
        let at = Vec2::new(420.0 + i as f32 * 44.0, 450.0);
        let mut button = commands.spawn((
            UiButton,
            TabIndex(0),
            Hovered::default(),
            Picture { sheet: sheet.clone(), frame: base, bounds: Some(base), at },
            absolute(at, Vec2::new(44.0, 30.0)),
            Pickable::default(),
            FocusControl,
            Outline::new(px(1), px(1), Color::NONE),
            ChildOf(root),
        ));
        if let Some(page) = page {
            button.insert(PageButton(page)).observe(select_page);
        } else {
            button.insert(InteractionDisabled);
            button.insert(Picture {
                sheet: sheet.clone(),
                frame: base + 3,
                bounds: Some(base),
                at,
            });
        }
    }

    let stats = panel(commands, root, Page::Statistics);
    label(commands, stats, Vec2::new(16.0, 10.0), 190.0, "ATTRIBUTES");
    for (i, attribute) in EnumSet::<Attribute>::all().iter().enumerate() {
        // Each row reads one typed domain value; there is no copied character view.
        let row = label(commands, stats, Vec2::new(16.0, 35.0 + i as f32 * 24.0), 195.0, "");
        commands.entity(row).insert(Field::Attribute(attribute));
    }
    let progress = label(commands, stats, Vec2::new(226.0, 10.0), 395.0, "");
    commands.entity(progress).insert(Field::Progress);
    let vitals = label(commands, stats, Vec2::new(226.0, 82.0), 395.0, "");
    commands.entity(vitals).insert(Field::Vitals);
    let conditions = label(commands, stats, Vec2::new(226.0, 128.0), 395.0, "");
    commands.entity(conditions).insert(Field::Conditions);

    let skills = panel(commands, root, Page::Skills);
    use Skill::*;
    for (title, at, entries) in [
        (
            "WEAPONS",
            Vec2::new(12.0, 8.0),
            &[
                Sword,
                Axe,
                Polearm,
                MaceAndFlail,
                Dagger,
                StaffAndWand,
                Shield,
                ModernWeapon,
                Bow,
                ThrowingAndSling,
                MartialArts,
            ][..],
        ),
        (
            "MISCELLANEOUS",
            Vec2::new(12.0, 182.0),
            &[LocksAndTraps, Stealth, Music, Pickpocket, Scouting][..],
        ),
        (
            "COMBAT",
            Vec2::new(226.0, 8.0),
            &[
                CloseCombat,
                RangedCombat,
                DualWeapons,
                CriticalStrike,
                Artifacts,
                Mythology,
                Communication,
                Engineering,
            ][..],
        ),
        (
            "EXPERT",
            Vec2::new(226.0, 144.0),
            &[PowerStrike, PowerCast, IronWill, IronSkin, Reflextion, SnakeSpeed, EagleEye][..],
        ),
        (
            "MAGIC",
            Vec2::new(440.0, 8.0),
            &[
                Wizardry,
                Divinity,
                Alchemy,
                Psionics,
                FireMagic,
                WaterMagic,
                AirMagic,
                EarthMagic,
                MentalMagic,
                DivineMagic,
            ][..],
        ),
    ] {
        label(commands, skills, at, 195.0, title);
        for (i, &skill) in entries.iter().enumerate() {
            let position = at + Vec2::new(0.0, 20.0 + i as f32 * 14.0);
            label(commands, skills, position, 145.0, &words(skill));
            let row = label(commands, skills, position + Vec2::new(145.0, 0.0), 53.0, "");
            commands.entity(row).insert(Field::Skill(skill));
        }
    }
    label(
        commands,
        skills,
        Vec2::new(440.0, 198.0),
        190.0,
        "Values: base / effective\nIncludes profession bonus.\nZero does not imply trained.",
    );
}

fn select_page(
    click: On<Activate>,
    buttons: Query<&PageButton>,
    mut screen: Single<&mut InventoryScreen>,
) {
    if let Ok(button) = buttons.get(click.entity) {
        screen.page = button.0;
    }
}

fn words(value: impl std::fmt::Debug) -> String {
    let value = format!("{value:?}");
    let mut words = String::new();
    for (i, c) in value.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            words.push(' ');
        }
        words.push(c);
    }
    words
}

pub(super) fn refresh(
    screen: Option<Single<Ref<InventoryScreen>>>,
    characters: Query<(Ref<Character>, Ref<Vitals>)>,
    mut pages: Query<(&Page, &mut Node, &mut Visibility)>,
    mut fields: Query<(Ref<Field>, &mut Text)>,
) {
    let Some(screen) = screen else { return };
    for (page, mut node, mut visibility) in &mut pages {
        let display = if *page == screen.page { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        // Native tab navigation follows inherited visibility, not layout display.
        visibility.set_if_neq(if *page == screen.page {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let Ok((profile, vitals)) = characters.get(screen.member) else { return };
    for (field, mut text) in &mut fields {
        let changed = match *field {
            Field::Attribute(_) | Field::Skill(_) | Field::Progress => profile.is_changed(),
            Field::Vitals => profile.is_changed() || vitals.is_changed(),
            Field::Conditions => vitals.is_changed(),
        };
        if !field.is_added() && !screen.is_changed() && !changed {
            continue;
        }
        let value = match *field {
            Field::Attribute(attribute) => {
                format!("{}: {}", words(attribute), profile.attribute(attribute))
            }
            Field::Skill(skill) => {
                format!("{}/{}", profile.skill_points[skill], profile.skill(skill))
            }
            Field::Progress => format!(
                "Level {} {:?}\nExperience: {}\nNext level: {}",
                profile.level,
                profile.profession,
                super::format_gold(profile.experience),
                super::format_gold(profile.experience_goal())
            ),
            Field::Vitals => format!(
                "Health: {} / {}\nStamina: {} / {}",
                vitals.hp,
                profile.max_hp(),
                vitals.stamina,
                profile.max_stamina()
            ),
            Field::Conditions => {
                let mut entries = Vec::new();
                if vitals.is_dead() {
                    entries.push("Dead".to_owned());
                }
                entries.extend(vitals.conditions.iter().map(
                    |(condition, lasting)| match lasting {
                        Lasting::Rounds(rounds) => {
                            format!(
                                "{} ({rounds} round{})",
                                words(condition),
                                if *rounds == 1 { "" } else { "s" }
                            )
                        }
                        Lasting::Indefinite => words(condition),
                    },
                ));
                format!(
                    "Conditions: {}",
                    if entries.is_empty() { "None".into() } else { entries.join(", ") }
                )
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

pub(super) fn style_buttons(
    screen: Option<Single<&InventoryScreen>>,
    mut buttons: Query<(&PageButton, &Hovered, Has<Pressed>, &mut Picture)>,
) {
    let Some(screen) = screen else { return };
    for (button, hovered, pressed, mut picture) in &mut buttons {
        let base = match button.0 {
            Page::Items => 0,
            Page::Skills => 15,
            Page::Statistics => 5,
        };
        let frame = base
            + if screen.page == button.0 || pressed {
                if hovered.0 { 2 } else { 4 }
            } else if hovered.0 {
                1
            } else {
                0
            };
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}

#[derive(Component, Clone, Default)]
pub(super) struct FocusControl;

/// Retail images keep their existing dimensions; focus is painted outside them.
pub(super) fn focus_controls(
    focus: Option<Res<InputFocus>>,
    mut controls: Query<(Entity, &mut Outline), With<FocusControl>>,
) {
    let focused = focus.as_ref().and_then(|f| f.get());
    for (entity, mut outline) in &mut controls {
        let colour = if focused == Some(entity) { GOLD } else { Color::NONE };
        if outline.color != colour {
            outline.color = colour;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Condition, Gender, Profession, Race};

    #[test]
    fn review_follows_damage_recovery_and_member_selection() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, refresh);
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let maximum = profile.max_hp();
        let actor = app.world_mut().spawn((Vitals::new(&profile), profile)).id();
        let other = Character::new("B", Race::Elf, Gender::Female, Profession::Mage);
        let other_maximum = other.max_hp();
        let other = app.world_mut().spawn((Vitals::new(&other), other)).id();
        let root = app
            .world_mut()
            .spawn(InventoryScreen { member: actor, pool_scroll: 0, page: Page::Statistics })
            .id();
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            build(&mut commands, root, default());
        });
        app.update();
        let contains = |world: &mut World, needle: &str| {
            world.query::<&Text>().iter(world).any(|text| text.0.contains(needle))
        };
        assert!(contains(app.world_mut(), &format!("Health: {maximum} / {maximum}")));
        {
            let mut vitals = app.world_mut().get_mut::<Vitals>(actor).unwrap();
            vitals.damage(1);
            vitals.inflict(Condition::Blind, Lasting::Rounds(1));
        }
        app.update();
        assert!(contains(app.world_mut(), &format!("Health: {} / {maximum}", maximum - 1)));
        assert!(contains(app.world_mut(), "Blind (1 round)"));
        {
            let mut vitals = app.world_mut().get_mut::<Vitals>(actor).unwrap();
            vitals.age_conditions(1);
            vitals.heal(1, maximum);
        }
        app.update();
        assert!(contains(app.world_mut(), "Conditions: None"));
        assert!(contains(app.world_mut(), &format!("Health: {maximum} / {maximum}")));
        app.world_mut().get_mut::<InventoryScreen>(root).unwrap().member = other;
        app.update();
        assert!(contains(app.world_mut(), &format!("Health: {other_maximum} / {other_maximum}")));
        assert!(contains(app.world_mut(), "Level 1 Mage"));
    }
}
