use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("cannot read the backlight {0}")]
    Unreadable(String),
    #[error("logind did not change the backlight: {0}")]
    Logind(#[from] zbus::Error),
    #[error("DDC/CI on /dev/i2c-{bus}: {message}")]
    Ddc { bus: u32, message: String },
    #[error("the output has no brightness hardware")]
    NoHardware,
    #[error("the blocking pool dropped the call")]
    Interrupted,
}
