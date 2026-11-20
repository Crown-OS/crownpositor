//! What is behind an output: a real monitor, a viewport of a nested window,
//! or a picture nobody plugged in. Only persistence and the primary fallback
//! ask; desktop membership is `Presence::Shown`.

const NESTED_PERSIST_VARIABLE: &str = "CROWN_NESTED_PERSIST";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// A connector on a GPU.
    Physical,
    /// A viewport of the nested window.
    Nested,
    /// `crownos_virtual_output_v1` outputs and `HEADLESS-1`.
    Virtual,
}

/// Whether nested outputs are saved to `compositor.ron`, which only a test of
/// the persistence itself wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedPersistence {
    Skip,
    Save,
}

impl NestedPersistence {
    pub fn from_environment() -> Self {
        match std::env::var(NESTED_PERSIST_VARIABLE).as_deref() {
            Ok("1") => Self::Save,
            _ => Self::Skip,
        }
    }
}

impl OutputKind {
    pub fn is_persisted(self, nested: NestedPersistence) -> bool {
        match self {
            Self::Physical => true,
            Self::Nested => nested == NestedPersistence::Save,
            Self::Virtual => false,
        }
    }
}
