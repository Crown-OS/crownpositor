//! Renders the lock screen for the current `lock` settings to a PNG, clock
//! and prompt both shown: `cargo run -p lockscreen --example snapshot -- out.png`.

use anyhow::{Context, Result};
use cosmic_text::FontSystem;
use lockscreen::{
    clock::ClockFace, layout::Live, lockscene::LockScene, paint::Painter, user::User,
};
use tiny_skia::{Color, Pixmap, PixmapPaint, Transform};

const SIZE: [f32; 2] = [1920.0, 1080.0];
const SCALE: f32 = 1.0;

fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .context("usage: snapshot <out.png>")?;
    let scene = LockScene {
        config: crownos_config::load(crownos_config::Lock::SECTION),
        theme: crownos_config::load(crownos_config::schema::Theme::SECTION),
        ..LockScene::default()
    };
    let face = ClockFace::now(&scene.config);
    let user = User::current()?;
    let live = Live {
        face: &face,
        user: Some(&user),
        password_chars: 6,
        checking: false,
        notice: Some(("Incorrect password", true)),
        caps_lock: true,
    };
    let mut painter = Painter::new(FontSystem::new());
    let sprites = painter.layout(&scene, live, &(), SIZE);

    let mut canvas =
        Pixmap::new((SIZE[0] * SCALE) as u32, (SIZE[1] * SCALE) as u32).context("canvas")?;
    canvas.fill(Color::from_rgba8(28, 30, 44, 255));
    for sprite in &sprites {
        let pad = (sprite.reach() * SCALE).ceil() as u32;
        let Some(pixmap) = painter.paint(sprite, SCALE, pad, &()) else {
            continue;
        };
        canvas.draw_pixmap(
            (sprite.rect.x * SCALE).round() as i32 - pad as i32,
            (sprite.rect.y * SCALE).round() as i32 - pad as i32,
            pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    canvas.save_png(&out)?;
    println!("{} sprites -> {out}", sprites.len());
    Ok(())
}
