use thiserror::Error;

#[derive(Debug, Error)]
pub enum SceneError {
    #[error("the scene is {got} bytes, over the {limit} byte limit")]
    TooLarge { got: usize, limit: usize },
    #[error("the scene did not decode: {0}")]
    Decode(#[from] postcard::Error),
    #[error("`{key}` is out of range")]
    OutOfRange { key: &'static str },
    #[error("`{key}` is not a valid format: {format:?}")]
    BadFormat { key: &'static str, format: String },
    #[error("the background {0:?} is neither an asset nor an absolute path")]
    BadBackground(String),
    #[error("`{0}` is not a theme color token")]
    BadToken(String),
    #[error("more than {limit} slots")]
    TooManySlots { limit: usize },
    #[error("slot id {0:?} is not `plugin:<id>/<widget>`")]
    BadSlotId(String),
    #[error("slot {0:?} appears twice")]
    DuplicateSlot(String),
    #[error("a tree has more than {limit} nodes")]
    TooManyNodes { limit: usize },
    #[error("a tree nests deeper than {limit}")]
    TooDeep { limit: usize },
    #[error("a text is longer than {limit} characters")]
    TextTooLong { limit: usize },
    #[error("lock trees are display-only, but one holds an interactive node")]
    Interactive,
    #[error("placeholder {0:?} is unknown or used outside the layout")]
    BadPlaceholder(String),
    #[error("the layout must hold exactly one `lock.secure_field`, it holds {0}")]
    SecureFieldCount(usize),
    #[error("image {0:?} is not an asset")]
    NotAnAsset(String),
    #[error("a style value is not finite or out of range")]
    BadStyle,
}
