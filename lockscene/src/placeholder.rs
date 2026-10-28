/// A part of the lock screen the compositor draws itself, named in a theme's
/// layout tree by a `Placeholder` node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placeholder<'a> {
    Clock,
    Date,
    SecureField,
    AuthPrompt,
    User,
    Power,
    Caps,
    /// A plugin slot by its full id, `plugin:<id>/<widget>`.
    Slot(&'a str),
}

impl<'a> Placeholder<'a> {
    pub fn parse(name: &'a str) -> Option<Self> {
        Some(match name {
            "lock.clock" => Self::Clock,
            "lock.date" => Self::Date,
            "lock.secure_field" => Self::SecureField,
            "lock.auth_prompt" => Self::AuthPrompt,
            "lock.user" => Self::User,
            "lock.power" => Self::Power,
            "lock.caps" => Self::Caps,
            _ => Self::Slot(name.strip_prefix("lock.slot:")?),
        })
    }

    /// Whether this part belongs to the password prompt rather than the clock
    /// screen, and so comes and goes with it.
    pub fn is_prompt(self) -> bool {
        matches!(
            self,
            Self::SecureField | Self::AuthPrompt | Self::User | Self::Caps | Self::Power
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_into_parts() {
        assert_eq!(Placeholder::parse("lock.clock"), Some(Placeholder::Clock));
        assert_eq!(
            Placeholder::parse("lock.slot:plugin:a.b/c"),
            Some(Placeholder::Slot("plugin:a.b/c"))
        );
        assert_eq!(Placeholder::parse("lock.nope"), None);
        assert!(Placeholder::SecureField.is_prompt());
        assert!(!Placeholder::Date.is_prompt());
    }
}
