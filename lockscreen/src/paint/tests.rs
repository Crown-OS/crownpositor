use crownos_config::schema::{
    Anchor, ClockArrangement, ClockFrame, ClockLook, Lock, LockLayout, LockSlotPosition, Placement,
};
use crownplugin_view::{Node, Tree};
use lockscene::{LockScene, Slot};

use super::*;
use crate::{
    clock::ClockFace,
    layout::{ClockGroup, GlyphPaint, Layer, Part, Rect},
    user::User,
};

const OUTPUT: [f32; 2] = [1920.0, 1080.0];

fn face() -> ClockFace {
    ClockFace {
        time: "12:34".into(),
        date: "Friday, 16 October".into(),
    }
}

fn user() -> User {
    User {
        login: "ada".into(),
        display_name: "Ada Lovelace".into(),
    }
}

fn live<'a>(face: &'a ClockFace, user: &'a User, chars: usize) -> Live<'a> {
    Live {
        face,
        user: Some(user),
        password_chars: chars,
        checking: false,
        notice: Some(("Incorrect password", true)),
        caps_lock: true,
    }
}

fn painter() -> Painter {
    Painter::new(FontSystem::new())
}

fn text_of(sprite: &Sprite) -> Option<&str> {
    match &sprite.content {
        Content::Text(run) => Some(&run.text),
        _ => None,
    }
}

/// The box around every glyph of the clock, or of one of its groups.
fn clock_box(sprites: &[Sprite], group: Option<ClockGroup>) -> Rect {
    let [left, top, right, bottom] = sprites
        .iter()
        .filter(|sprite| sprite.clock_group().is_some())
        .filter(|sprite| group.is_none() || sprite.clock_group() == group)
        .fold(
            [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            |[l, t, r, b], sprite| {
                let rect = sprite.rect;
                [
                    l.min(rect.x),
                    t.min(rect.y),
                    r.max(rect.x + rect.width),
                    b.max(rect.y + rect.height),
                ]
            },
        );
    assert!(left < right, "no clock glyphs");
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn clock_scene(clock: ClockLook) -> LockScene {
    LockScene {
        config: Lock {
            clock,
            ..Lock::default()
        },
        ..LockScene::default()
    }
}

fn glyphs(sprites: &[Sprite]) -> String {
    sprites
        .iter()
        .filter_map(|sprite| match &sprite.content {
            Content::Glyph(run) if !matches!(run.paint, GlyphPaint::Shadow { .. }) => Some(run.ch),
            _ => None,
        })
        .collect()
}

#[test]
fn the_stock_layout_splits_clock_and_prompt() {
    let (face, user) = (face(), user());
    let sprites = painter().layout(&LockScene::default(), live(&face, &user, 3), &(), OUTPUT);

    assert_eq!(glyphs(&sprites), "12:34");
    assert!(
        sprites
            .iter()
            .filter(|sprite| sprite.clock_group().is_some())
            .all(|sprite| sprite.layer == Layer::Clock)
    );
    assert!((clock_box(&sprites, None).centre()[0] - OUTPUT[0] / 2.0).abs() < 2.0);

    let field = sprites
        .iter()
        .find(|sprite| sprite.part == Some(Part::SecureField))
        .expect("a field");
    assert_eq!(field.layer, Layer::Prompt);
    assert!((field.rect.centre()[1] - OUTPUT[1] / 2.0).abs() < 120.0);
    for words in ["Ada Lovelace", "Incorrect password", "Caps Lock is on"] {
        let sprite = sprites
            .iter()
            .find(|sprite| text_of(sprite) == Some(words))
            .unwrap_or_else(|| panic!("{words}"));
        assert_eq!(sprite.layer, Layer::Prompt, "{words}");
    }
}

#[test]
fn placement_settings_move_things() {
    let (face, user) = (face(), user());
    let scene = LockScene {
        config: Lock {
            layout: LockLayout::Left,
            auth_field: Placement {
                anchor: Anchor::Bottom,
                offset: [0.0, -40.0],
            },
            ..Lock::default()
        },
        ..LockScene::default()
    };
    let sprites = painter().layout(&scene, live(&face, &user, 0), &(), OUTPUT);
    assert!(clock_box(&sprites, None).x < OUTPUT[0] * 0.2);
    let field = sprites
        .iter()
        .find(|sprite| sprite.part == Some(Part::SecureField))
        .expect("a field");
    assert!(field.rect.y > OUTPUT[1] * 0.6);
}

#[test]
fn slots_sit_below_the_clock_and_ids_stay_put() {
    let (face, user) = (face(), user());
    let scene = LockScene {
        slots: vec![Slot {
            id: "plugin:a.b/c".into(),
            position: LockSlotPosition::BelowClock,
            offset: [0.0, 0.0],
            tree: Tree {
                rev: 0,
                root: Node::Card {
                    style: Vec::new(),
                    children: vec![Node::Text {
                        style: Vec::new(),
                        text: "Good morning".into(),
                    }],
                },
            },
        }],
        ..LockScene::default()
    };
    let mut painter = painter();
    let first = painter.layout(&scene, live(&face, &user, 1), &(), OUTPUT);
    let clock = clock_box(&first, None);
    let greeting = first
        .iter()
        .find(|sprite| text_of(sprite) == Some("Good morning"))
        .expect("the slot");
    assert_eq!(greeting.layer, Layer::Clock);
    assert!(greeting.rect.y > clock.y + clock.height);
    assert!(
        first
            .iter()
            .any(|sprite| matches!(sprite.content, Content::Panel { .. }))
    );

    let second = painter.layout(&scene, live(&face, &user, 2), &(), OUTPUT);
    let id = |sprites: &[Sprite], words: &str| {
        sprites
            .iter()
            .find(|sprite| text_of(sprite) == Some(words))
            .map(|sprite| sprite.id)
    };
    assert_eq!(id(&first, "Good morning"), id(&second, "Good morning"));
}

#[test]
fn every_sprite_paints() {
    let (face, user) = (face(), user());
    let mut painter = painter();
    let sprites = painter.layout(&LockScene::default(), live(&face, &user, 4), &(), OUTPUT);
    assert!(!sprites.is_empty());
    for sprite in &sprites {
        let pixmap = painter.paint(sprite, 1.5, 8, &()).expect("paints");
        assert_eq!(
            pixmap.width(),
            (sprite.rect.width * 1.5).ceil() as u32 + 16,
            "{sprite:?}"
        );
        assert!(
            pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0),
            "{sprite:?} drew nothing"
        );
    }
}

#[test]
fn a_changed_digit_keeps_its_place_and_id() {
    let user = user();
    let before = face();
    let after = ClockFace {
        time: "12:35".into(),
        ..face()
    };
    let mut painter = painter();
    let scene = LockScene::default();
    let first = painter.layout(&scene, live(&before, &user, 0), &(), OUTPUT);
    let second = painter.layout(&scene, live(&after, &user, 0), &(), OUTPUT);
    let digits = |sprites: &[Sprite]| -> Vec<(u64, char, f32)> {
        sprites
            .iter()
            .filter_map(|sprite| match &sprite.content {
                Content::Glyph(run) => Some((sprite.id, run.ch, sprite.rect.x)),
                _ => None,
            })
            .collect()
    };
    let (first, second) = (digits(&first), digits(&second));
    assert_eq!(first.len(), second.len());
    let changed: Vec<_> = first
        .iter()
        .zip(&second)
        .filter(|(old, new)| old.1 != new.1)
        .collect();
    assert_eq!(changed.len(), 1);
    let (old, new) = changed[0];
    assert_eq!((old.0, old.2), (new.0, new.2), "same cell, same place");
}

#[test]
fn stacked_puts_minutes_under_hours() {
    let (face, user) = (face(), user());
    let scene = clock_scene(ClockLook {
        arrangement: ClockArrangement::Stacked,
        ..ClockLook::default()
    });
    let sprites = painter().layout(&scene, live(&face, &user, 0), &(), OUTPUT);
    assert_eq!(glyphs(&sprites), "1234", "no separator when stacked");
    let hours = clock_box(&sprites, Some(ClockGroup::Hours));
    let minutes = clock_box(&sprites, Some(ClockGroup::Minutes));
    assert!(minutes.y >= hours.y + hours.height);
}

#[test]
fn stretch_makes_the_digits_taller_not_wider() {
    let (face, user) = (face(), user());
    let mut painter = painter();
    let plain = clock_box(
        &painter.layout(&LockScene::default(), live(&face, &user, 0), &(), OUTPUT),
        None,
    );
    let tall_scene = clock_scene(ClockLook {
        stretch: 2.0,
        ..ClockLook::default()
    });
    let tall = clock_box(
        &painter.layout(&tall_scene, live(&face, &user, 0), &(), OUTPUT),
        None,
    );
    assert!((tall.height / plain.height - 2.0).abs() < 0.05);
    assert!((tall.width - plain.width).abs() < 1.0);
}

#[test]
fn the_frame_moves_the_clock_and_its_groups() {
    let (face, user) = (face(), user());
    let mut painter = painter();
    let rest = painter.layout(&LockScene::default(), live(&face, &user, 0), &(), OUTPUT);
    let moved_scene = clock_scene(ClockLook {
        frame: ClockFrame {
            offset: [30.0, -10.0],
            minutes: [0.0, 50.0],
            ..ClockFrame::default()
        },
        ..ClockLook::default()
    });
    let moved = painter.layout(&moved_scene, live(&face, &user, 0), &(), OUTPUT);
    let shift = |group| {
        let (a, b) = (
            clock_box(&rest, Some(group)),
            clock_box(&moved, Some(group)),
        );
        [b.x - a.x, b.y - a.y]
    };
    assert_eq!(shift(ClockGroup::Hours), [30.0, -10.0]);
    assert_eq!(shift(ClockGroup::Minutes), [30.0, 40.0]);
}

#[test]
fn a_shadow_sits_under_each_face_and_paints() {
    let (face, user) = (face(), user());
    let scene = clock_scene(ClockLook {
        shadow: 1.0,
        ..ClockLook::default()
    });
    let mut painter = painter();
    let sprites = painter.layout(&scene, live(&face, &user, 0), &(), OUTPUT);
    let shadow = sprites
        .iter()
        .position(|sprite| {
            matches!(&sprite.content, Content::Glyph(run) if matches!(run.paint, GlyphPaint::Shadow { .. }))
        })
        .expect("a shadow");
    let face_sprite = &sprites[shadow + 1];
    assert_eq!(
        face_sprite.rect, sprites[shadow].rect,
        "drawn over its shadow"
    );
    let pad = (sprites[shadow].reach() * 2.0).ceil() as u32;
    let pixmap = painter
        .paint(&sprites[shadow], 2.0, pad, &())
        .expect("paints");
    let edge = pixmap.pixel(1, pixmap.height() / 2).expect("inside");
    assert_eq!(edge.alpha(), 0, "the blur stays inside its padding");
    assert!(pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0));
}
