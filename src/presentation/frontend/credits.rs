//! The original credits, scrolled over the retail background.
use super::MenuPage;
use crate::content::ui::{CreditLine, Credits, SpriteSheet};
use crate::presentation::screen_art::{BitmapText, absolute, spawn_picture};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

#[derive(Component, Clone, Default)]
pub(super) struct CreditsInput;
#[derive(InputAction)]
#[action_output(bool)]
struct LeaveCredits;

#[derive(Component)]
pub(super) struct CreditsView {
    text: Handle<Credits>,
    title: Handle<SpriteSheet>,
    detail: Handle<SpriteSheet>,
}
#[derive(Component)]
pub(super) struct Built;
#[derive(Component)]
pub(super) struct Roll {
    height: f32,
    elapsed: f32,
}

pub(super) fn enter(mut commands: Commands, assets: Res<AssetServer>) {
    let text = assets.load("retail://data/options/credits.txt");
    let title = assets.load("retail://data/fonts/opt_title_font.sti");
    let detail = assets.load("retail://data/fonts/opt_detail_font.sti");
    let background = assets.load("retail://data/options/intro_bg_credits.sti");
    let root = commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        BackgroundColor(Color::BLACK)
        GlobalZIndex(50)
        CreditsInput
        ContextPriority::<CreditsInput>::new(1)
        Actions<CreditsInput> [
            Action::<LeaveCredits>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(Binding::AnyKey) ]
        ]
        on(|_: On<Start<LeaveCredits>>, mut next: ResMut<NextState<MenuPage>>| { next.set(MenuPage::Main); })
        on(|_: On<PointerClick>, mut next: ResMut<NextState<MenuPage>>| { next.set(MenuPage::Main); })
    }).insert(DespawnOnExit(MenuPage::Credits)).id();
    let screen = commands
        .spawn((
            CreditsView { text, title, detail },
            Node { overflow: Overflow::clip(), ..absolute(Vec2::ZERO, Vec2::new(640., 480.)) },
            Pickable::default(),
            ChildOf(root),
        ))
        .id();
    spawn_picture(&mut commands, screen, background, 0, Vec2::ZERO);
}

fn line(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    value: &str,
    at: Vec2,
    width: f32,
    justify: bevy::text::Justify,
) {
    commands.spawn((
        BitmapText { font, value: value.to_owned(), at, width, justify, ..default() },
        ChildOf(parent),
    ));
}

pub(super) fn populate(
    mut commands: Commands,
    views: Query<(Entity, &CreditsView), Without<Built>>,
    credits: Res<Assets<Credits>>,
) {
    for (entity, view) in &views {
        let Some(credits) = credits.get(&view.text) else { continue };
        let content = commands
            .spawn((
                Node { position_type: PositionType::Absolute, top: px(479), ..default() },
                Pickable::IGNORE,
                ChildOf(entity),
            ))
            .id();
        let mut y = 0.;
        for entry in &credits.0 {
            match entry {
                CreditLine::Heading(value) => line(
                    &mut commands,
                    content,
                    view.title.clone(),
                    value,
                    Vec2::new(0., y),
                    640.,
                    bevy::text::Justify::Center,
                ),
                CreditLine::Centred(value) => line(
                    &mut commands,
                    content,
                    view.detail.clone(),
                    value,
                    Vec2::new(0., y),
                    640.,
                    bevy::text::Justify::Center,
                ),
                CreditLine::Pair(left, right) => {
                    line(
                        &mut commands,
                        content,
                        view.detail.clone(),
                        left,
                        Vec2::new(0., y),
                        310.,
                        bevy::text::Justify::Right,
                    );
                    line(
                        &mut commands,
                        content,
                        view.detail.clone(),
                        right,
                        Vec2::new(330., y),
                        0.,
                        bevy::text::Justify::Left,
                    );
                }
                CreditLine::Blank => {}
            }
            y += if matches!(entry, CreditLine::Heading(_)) { 25. } else { 20. };
        }
        commands.entity(content).insert(Roll { height: y, elapsed: 0. });
        commands.entity(entity).insert(Built);
    }
}

pub(super) fn scroll(
    time: Res<Time<Real>>,
    mut rolls: Query<(&mut Roll, &mut Node), Without<CreditsView>>,
    mut next: ResMut<NextState<MenuPage>>,
) {
    for (mut roll, mut node) in &mut rolls {
        roll.elapsed += time.delta_secs();
        let offset = (roll.elapsed / 0.035).floor();
        node.top = px(479. - offset);
        if offset > 479. + roll.height {
            next.set(MenuPage::Main);
        }
    }
}
