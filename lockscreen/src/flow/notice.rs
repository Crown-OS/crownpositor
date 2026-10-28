use std::{borrow::Cow, time::Instant};

const SERVICE_TEXT_CHARS: usize = 200;

/// The line under the password field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Notice {
    #[default]
    None,
    Checking,
    Rejected,
    /// Too many failures: input is refused until then.
    Wait {
        until: Instant,
    },
    /// Something the authentication service wants the user to read.
    Service(String),
}

impl Notice {
    pub fn service(text: &str) -> Self {
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(SERVICE_TEXT_CHARS)
            .collect();
        let clean = clean.trim();
        if clean.is_empty() {
            Self::None
        } else {
            Self::Service(clean.to_owned())
        }
    }

    pub fn text(&self, now: Instant) -> Option<Cow<'_, str>> {
        Some(match self {
            Self::None => return None,
            Self::Checking => Cow::Borrowed("Checking…"),
            Self::Rejected => Cow::Borrowed("Incorrect password"),
            Self::Wait { until } => {
                let seconds = until.saturating_duration_since(now).as_secs() + 1;
                Cow::Owned(format!("Too many attempts. Try again in {seconds} s"))
            }
            Self::Service(text) => Cow::Borrowed(text),
        })
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Rejected | Self::Wait { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_text_is_cleaned_and_bounded() {
        let notice = Notice::service("  locked\n\x1b[31mfor 5 min ");
        assert_eq!(notice, Notice::Service("locked[31mfor 5 min".into()));
        let Notice::Service(long) = Notice::service(&"x".repeat(1000)) else {
            panic!("kept");
        };
        assert_eq!(long.chars().count(), SERVICE_TEXT_CHARS);
        assert_eq!(Notice::service(" \n "), Notice::None);
    }
}
