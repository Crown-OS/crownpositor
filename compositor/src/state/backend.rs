use crate::backend::winit::WinitState;

pub enum BackendState {
    Unset,
    Winit(Box<WinitState>),
}

impl BackendState {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self::Unset)
    }

    pub fn winit(&mut self) -> Option<&mut WinitState> {
        match self {
            Self::Winit(winit) => Some(winit),
            Self::Unset => None,
        }
    }
}
