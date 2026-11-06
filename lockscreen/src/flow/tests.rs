use super::*;
use crate::transition::{LayerStyle, Moments, PROMPT_BLUR};

fn flow(now: Instant) -> LockFlow {
    LockFlow::new(&Lock::default(), now)
}

fn later(now: Instant, millis: u64) -> Instant {
    now + Duration::from_millis(millis)
}

fn type_text(flow: &mut LockFlow, text: &str, now: Instant) {
    for c in text.chars() {
        let _ = flow.key(KeyInput::Char(c), now);
    }
}

#[test]
fn only_an_accepted_check_unlocks() {
    let now = Instant::now();
    let mut flow = flow(now);
    type_text(&mut flow, "hunter2", now);
    let Response::Authenticate(password) = flow.key(KeyInput::Submit, now) else {
        panic!("submitting a password checks it");
    };
    assert_eq!(password.as_bytes(), b"hunter2");
    assert_eq!(
        flow.password_chars(),
        7,
        "the dots stay while it is checked"
    );
    assert!(flow.is_checking() && !flow.is_unlocking());
    assert!(matches!(
        flow.key(KeyInput::Char('x'), now),
        Response::Ignored
    ));

    flow.finish(AuthOutcome::Rejected, now);
    assert!(!flow.is_unlocking());
    assert_eq!(flow.notice(), &Notice::Rejected);
    assert_eq!(flow.password_chars(), 0, "a rejected password is gone");

    type_text(&mut flow, "right", now);
    let _ = flow.key(KeyInput::Submit, now);
    flow.finish(AuthOutcome::Accepted, now);
    assert!(flow.is_unlocking());
    assert!(!flow.is_finished(now));
    assert!(flow.is_finished(later(now, 2000)));
    assert!(matches!(
        flow.key(KeyInput::Char('x'), now),
        Response::Ignored
    ));
}

#[test]
fn an_empty_password_is_never_checked() {
    let now = Instant::now();
    let mut flow = flow(now);
    assert!(matches!(flow.key(KeyInput::Submit, now), Response::Redraw));
    assert!(!flow.is_checking());
}

#[test]
fn repeated_failures_back_off() {
    assert_eq!(backoff(1), None);
    assert_eq!(backoff(2), None);
    assert_eq!(backoff(3), Some(Duration::from_secs(2)));
    assert_eq!(backoff(4), Some(Duration::from_secs(4)));
    assert_eq!(backoff(200), Some(BACKOFF_CAP));

    let now = Instant::now();
    let mut flow = flow(now);
    for _ in 0..3 {
        type_text(&mut flow, "nope", now);
        let _ = flow.key(KeyInput::Submit, now);
        flow.finish(AuthOutcome::Rejected, now);
    }
    assert!(matches!(flow.notice(), Notice::Wait { .. }));
    type_text(&mut flow, "x", now);
    assert_eq!(flow.password_chars(), 0, "typing waits out the backoff");
    assert!(flow.poll(later(now, 2001)));
    type_text(&mut flow, "x", later(now, 2001));
    assert_eq!(flow.password_chars(), 1);
}

#[test]
fn the_prompt_times_out_and_forgets_what_was_typed() {
    let now = Instant::now();
    let config = Lock {
        prompt_timeout_secs: 5,
        ..Lock::default()
    };
    let mut flow = LockFlow::new(&config, now);
    type_text(&mut flow, "half", now);
    assert_eq!(flow.next_deadline(), Some(now + Duration::from_secs(5)));
    assert!(!flow.poll(later(now, 4000)));
    assert!(flow.poll(later(now, 5000)));
    assert_eq!(flow.password_chars(), 0);
    let settled = later(now, 9000);
    assert!(!flow.frame(settled).prompt.is_visible());
    assert!(flow.frame(settled).clock.is_visible());
}

#[test]
fn layers_trade_places_on_reveal() {
    let now = Instant::now();
    let mut flow = flow(now);
    let rested = later(now, 1000);
    let frame = flow.frame(rested);
    assert_eq!(frame.clock, LayerStyle::REST);
    assert!(!frame.prompt.is_visible());
    assert_eq!(frame.wallpaper, LayerStyle::REST);

    let _ = flow.pointer(rested);
    let revealed = flow.frame(later(rested, 1000));
    assert_eq!(revealed, Moments::PROMPT.frame(0.0));

    let _ = flow.key(KeyInput::Cancel, later(rested, 1000));
    let hidden = flow.frame(later(rested, 2000));
    assert_eq!(hidden, Moments::CLOCK.frame(0.0));
}

#[test]
fn locking_covers_the_desktop_at_once_and_lands_at_rest() {
    let now = Instant::now();
    let flow = flow(now);
    let start = flow.frame(now);
    assert!(!start.clock.is_visible());
    assert_eq!(start.wallpaper.opacity, 1.0);
    assert!(flow.is_animating(later(now, 100)));
    let landed = later(now, 600);
    assert!(!flow.is_animating(landed));
    assert_eq!(flow.frame(landed), Moments::CLOCK.frame(0.0));
}

#[test]
fn turning_back_mid_reveal_is_continuous() {
    let now = Instant::now();
    let mut flow = LockFlow::settled(&Lock::default(), now);
    let _ = flow.pointer(now);
    let turn = later(now, 150);
    let before = flow.frame(turn);
    assert!(before.prompt.is_visible() && before.clock.is_visible());
    let _ = flow.key(KeyInput::Cancel, turn);
    assert_eq!(flow.frame(turn), before, "no jump where it turns");
    let step = flow.frame(later(now, 158));
    let rise = Moments::PROMPT.frame(0.0).clock.offset[1];
    for (was, is) in [
        (before.clock.opacity, step.clock.opacity),
        (before.clock.offset[1] / rise, step.clock.offset[1] / rise),
        (
            before.wallpaper.blur / PROMPT_BLUR,
            step.wallpaper.blur / PROMPT_BLUR,
        ),
    ] {
        assert!((was - is).abs() < 0.1, "{was} -> {is}");
    }
    let settled = later(turn, 1000);
    assert!(!flow.is_animating(settled));
    assert_eq!(flow.frame(settled), Moments::CLOCK.frame(0.0));
}

#[test]
fn a_settled_lock_has_nothing_left_to_animate() {
    let now = Instant::now();
    let flow = LockFlow::settled(&Lock::default(), now);
    assert!(!flow.is_animating(now));
    assert_eq!(flow.frame(now), Moments::CLOCK.frame(0.0));
}

#[test]
fn a_finished_dismissal_is_invisible() {
    let now = Instant::now();
    let mut flow = LockFlow::settled(&Lock::default(), now);
    type_text(&mut flow, "pw", now);
    let typed = later(now, 1000);
    let _ = flow.key(KeyInput::Submit, typed);
    flow.finish(AuthOutcome::Accepted, typed);
    assert_eq!(
        flow.frame(typed),
        Moments::PROMPT.frame(0.0),
        "starts at rest"
    );
    assert!(flow.is_animating(later(typed, 100)));
    assert!(!flow.is_finished(later(typed, 200)));
    let gone_at = later(typed, 600);
    assert!(flow.is_finished(gone_at));
    let gone = flow.frame(gone_at);
    assert!(!gone.clock.is_visible() && !gone.prompt.is_visible());
    assert!(!gone.wallpaper.is_visible());
    assert_eq!(gone.darkening(0.5), 0.0);
}
