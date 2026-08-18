use std::{collections::VecDeque, time::Duration};

const HISTORY_LIMIT: Duration = Duration::from_millis(150);
const DECELERATION_TOUCHPAD: f64 = 0.997;

#[derive(Debug, Clone, Copy)]
pub struct SwipeEvent {
    delta: f64,
    timestamp: Duration,
}

#[derive(Debug, Clone)]
pub enum Fingers {
    Two,
    Three,
    Four,
    Five,
}

#[derive(Debug, Clone)]
pub enum SwipeGesture {
    Tap(Fingers),
    LeftToRight(Fingers),
    RightToLeft(Fingers),
    BottomToTop(Fingers),
    TopToBottom(Fingers),
}

#[derive(Debug, Clone)]
pub enum SwipeAction {
    NextWorkspace,
    PrevWorkspace,
    OpenWorkspaceView,
    CloseWorkspaceView,
}

// TODO: Can be a enum?
#[derive(Debug, Clone)]
pub struct GestureState {
    pub fingers: Option<Fingers>,
    pub gesture: Option<SwipeGesture>,
    pub action: Option<SwipeAction>,
    pub delta: f64,
    pub history: VecDeque<SwipeEvent>,
}

impl GestureState {
    pub fn new() -> Self {
        GestureState {
            fingers: None,
            gesture: None,
            action: None,
            delta: 0.0,
            history: VecDeque::new(),
        }
    }
}

impl Default for GestureState {
    fn default() -> Self {
        GestureState::new()
    }
}
