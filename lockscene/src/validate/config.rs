use chrono::format::{Item, StrftimeItems};
use crownos_config::schema::{
    Effect, Lock, LockBackground, Theme, Transition, theme::is_color_token, theme::parse_hex,
};

use crate::{AssetId, Limits, SceneError};

pub(super) fn check(config: &Lock, theme: &Theme) -> Result<(), SceneError> {
    unit("lock.dim", config.dim)?;
    unit("lock.blur", config.blur)?;
    strftime("lock.time_format", &config.time_format)?;
    strftime("lock.date_format", &config.date_format)?;
    if config
        .font
        .as_ref()
        .is_some_and(|font| font.chars().count() > Limits::FONT_CHARS)
    {
        return Err(SceneError::OutOfRange { key: "lock.font" });
    }
    background(&config.background)?;
    check_offset(config.auth_field.offset)?;
    for slot in &config.slots {
        check_offset(slot.offset)?;
    }
    if config.prompt_timeout_secs > Limits::PROMPT_TIMEOUT_SECS {
        return Err(SceneError::OutOfRange {
            key: "lock.prompt_timeout_secs",
        });
    }
    let transitions = &config.transitions;
    for transition in [
        &transitions.appear,
        &transitions.reveal,
        &transitions.dismiss,
    ] {
        check_transition(transition)?;
    }
    check_theme(theme)
}

pub(super) fn check_offset(offset: [f32; 2]) -> Result<(), SceneError> {
    if offset
        .iter()
        .all(|value| value.is_finite() && value.abs() <= Limits::OFFSET)
    {
        Ok(())
    } else {
        Err(SceneError::OutOfRange { key: "offset" })
    }
}

fn unit(key: &'static str, value: f32) -> Result<(), SceneError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(SceneError::OutOfRange { key })
    }
}

fn strftime(key: &'static str, format: &str) -> Result<(), SceneError> {
    let valid = format.chars().count() <= Limits::FORMAT_CHARS
        && StrftimeItems::new(format).all(|item| item != Item::Error);
    if valid {
        Ok(())
    } else {
        Err(SceneError::BadFormat {
            key,
            format: format.to_owned(),
        })
    }
}

fn background(background: &LockBackground) -> Result<(), SceneError> {
    match background {
        LockBackground::Desktop => Ok(()),
        LockBackground::Image(src) => {
            let absolute = src.starts_with('/') && src.chars().count() <= Limits::PATH_CHARS;
            if absolute || AssetId::from_src(src).is_some() {
                Ok(())
            } else {
                Err(SceneError::BadBackground(src.clone()))
            }
        }
        LockBackground::Token(token) if is_color_token(token) => Ok(()),
        LockBackground::Token(token) => Err(SceneError::BadToken(token.clone())),
    }
}

fn check_transition(transition: &Transition) -> Result<(), SceneError> {
    let key = "lock.transitions";
    if transition.effects.len() > Limits::EFFECTS || transition.duration_ms > Limits::DURATION_MS {
        return Err(SceneError::OutOfRange { key });
    }
    let bounded = transition.effects.iter().all(|effect| match *effect {
        Effect::Fade => true,
        Effect::Blur { radius } => (0.0..=Limits::BLUR_RADIUS).contains(&radius),
        Effect::Slide { distance, .. } => (0.0..=Limits::SLIDE_DISTANCE).contains(&distance),
        Effect::Zoom { from } => Limits::ZOOM.contains(&from),
    });
    if bounded {
        Ok(())
    } else {
        Err(SceneError::OutOfRange { key })
    }
}

fn check_theme(theme: &Theme) -> Result<(), SceneError> {
    for (slot, color) in theme.dark.iter().chain(&theme.light) {
        if !is_color_token(slot) {
            return Err(SceneError::BadToken(slot.clone()));
        }
        if parse_hex(color).is_none() {
            return Err(SceneError::OutOfRange { key: "theme.color" });
        }
    }
    let radii = theme.radii;
    let font = &theme.font;
    let sane_radii = [radii.sm, radii.md, radii.lg, radii.xl]
        .iter()
        .all(|radius| (0.0..=Limits::LENGTH).contains(radius));
    let sane_font = (0.5..=3.0).contains(&font.scale)
        && font.family.chars().count() <= Limits::FONT_CHARS
        && font.mono.chars().count() <= Limits::FONT_CHARS;
    if sane_radii && sane_font {
        Ok(())
    } else {
        Err(SceneError::OutOfRange { key: "theme" })
    }
}
