//! Retail character-creation skills page.
use super::{Action, Creation, Page, absolute, creator_stats};
use crate::content::characters::Skill;
use crate::content::ui::SpriteSheet;
use crate::presentation::screen_art::spawn_picture;
use crate::presentation::screen_art::visible_if;
use bevy::prelude::*;
use enumset::EnumSet;

#[derive(Component)]
pub(super) struct SkillRow(Skill);

fn category(skill: Skill) -> usize {
    match skill as usize {
        0..=9 | 14 => 0,
        10..=13 | 15 => 1,
        16..=23 => 2,
        24..=33 => 3,
        _ => 4,
    }
}

pub(super) fn build(commands: &mut Commands, root: Entity, assets: &AssetServer) {
    creator_stats::frame(commands, root, assets, Page::Skills);
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let background: Handle<SpriteSheet> = load("char generation/cg_skills");
    let buttons: Handle<SpriteSheet> = load("char generation/cg_buttons");
    let bottom: Handle<SpriteSheet> = load("char generation/cg_bottombuttons");
    let font: Handle<SpriteSheet> = load("fonts/wiz_text_font");
    let icons: Handle<SpriteSheet> = load("review/reviewskillsicons");
    spawn_picture(commands, root, background.clone(), 0, Vec2::new(195., 43.));
    // No expert skills can be reached by the currently modelled creation rules.
    spawn_picture(commands, root, background.clone(), 1, Vec2::new(195., 323.));
    for (frame, x, y) in [(2, 422., 50.), (3, 422., 245.), (0, 207., 50.), (1, 207., 175.)] {
        spawn_picture(commands, root, icons.clone(), frame, Vec2::new(x, y));
    }
    use bevy::text::Justify::{Left, Right};
    for skill in EnumSet::<Skill>::all().iter().take(34) {
        let row = commands
            .spawn((
                SkillRow(skill),
                absolute(Vec2::ZERO, Vec2::new(186., 14.)),
                Visibility::Hidden,
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id();
        spawn_picture(commands, row, background.clone(), 2, Vec2::ZERO);
        creator_stats::label(
            commands,
            row,
            font.clone(),
            Vec2::new(5., 1.),
            104.,
            Left,
            creator_stats::Value::SkillName(skill),
        );
        creator_stats::label(
            commands,
            row,
            font.clone(),
            Vec2::new(109., 1.),
            23.,
            Right,
            creator_stats::Value::Skill(skill),
        );
        creator_stats::label(
            commands,
            row,
            font.clone(),
            Vec2::new(151., 1.),
            15.,
            Right,
            creator_stats::Value::AllocatedSkill(skill),
        );
        creator_stats::button(
            commands,
            row,
            buttons.clone(),
            Vec2::new(136., 1.),
            25,
            Action::Skill(skill, false),
        );
        creator_stats::button(
            commands,
            row,
            buttons.clone(),
            Vec2::new(170., 1.),
            30,
            Action::Skill(skill, true),
        );
    }
    creator_stats::button(commands, root, bottom.clone(), Vec2::new(464., 450.), 4, Action::Cancel);
    creator_stats::button(
        commands,
        root,
        bottom.clone(),
        Vec2::new(508., 450.),
        20,
        Action::ResetSkills,
    );
    creator_stats::button(
        commands,
        root,
        bottom.clone(),
        Vec2::new(552., 450.),
        12,
        Action::Page(Page::Identity),
    );
    creator_stats::button(
        commands,
        root,
        bottom,
        Vec2::new(596., 450.),
        8,
        Action::Page(Page::Review),
    );
}

pub(super) fn refresh(
    session: Option<Single<Ref<Creation>>>,
    mut rows: Query<(&SkillRow, &mut Node, &mut Visibility)>,
    added: Query<(), Added<SkillRow>>,
) {
    let Some(session) = session else { return };
    if !session.is_changed() && added.is_empty() {
        return;
    }
    let Some(draft) = session.mode.draft() else { return };
    let profile = draft.preview();
    for (row, mut node, mut visibility) in &mut rows {
        let shown = draft.skill_available(row.0) || profile.skill_points[row.0] > 0;
        visibility.set_if_neq(visible_if(shown));
        crate::presentation::screen_art::display_if(&mut node, shown);
        if shown {
            let group = category(row.0);
            let index = EnumSet::<Skill>::all()
                .iter()
                .take(row.0 as usize)
                .filter(|&skill| {
                    category(skill) == group
                        && (draft.skill_available(skill) || profile.skill_points[skill] > 0)
                })
                .count();
            let (x, y) =
                [(444., 53.), (444., 248.), (229., 53.), (229., 178.), (444., 331.)][group];
            let (left, top) = (px(x), px(y + index as f32 * 14.));
            if node.left != left {
                node.left = left;
            }
            if node.top != top {
                node.top = top;
            }
        }
    }
}
