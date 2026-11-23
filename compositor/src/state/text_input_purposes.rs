//! The content purpose that each `zwp_text_input_v3` object declared with its
//! last commit. Smithay passes the purpose only to an input method client, so
//! the dispatch of [`State`](super::State) records it here before smithay
//! handles the request.

use std::collections::HashMap;

use smithay::reexports::{
    wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
        self, ContentPurpose, ZwpTextInputV3,
    },
    wayland_server::{Resource, backend::ObjectId},
};

#[derive(Debug, Default)]
struct PendingDeclaration {
    enable: Option<bool>,
    purpose: Option<Option<ContentPurpose>>,
}

#[derive(Debug, Default)]
struct TextInputDeclarations {
    pending: PendingDeclaration,
    current: AppliedTextInput,
}

/// What each text input object declared, as the protocol applies it.
#[derive(Debug, Default)]
pub struct TextInputPurposes {
    declarations: HashMap<ObjectId, TextInputDeclarations>,
}

/// The state that one text input object applied with its last commit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppliedTextInput {
    pub enabled: bool,
    pub purpose: Option<ContentPurpose>,
    /// How many commits enabled the text input: one more each time a field takes the focus.
    pub enables: u32,
}

impl AppliedTextInput {
    pub fn is_password(&self) -> bool {
        self.enabled && self.purpose == Some(ContentPurpose::Password)
    }

    pub fn is_terminal(&self) -> bool {
        self.enabled && self.purpose == Some(ContentPurpose::Terminal)
    }

    fn apply(&mut self, pending: PendingDeclaration) {
        match pending.enable {
            Some(true) => {
                self.enabled = true;
                self.purpose = pending.purpose.flatten();
                self.enables = self.enables.wrapping_add(1);
            }
            Some(false) => {
                self.enabled = false;
                self.purpose = None;
            }
            None => {
                if let Some(purpose) = pending.purpose {
                    self.purpose = purpose;
                }
            }
        }
    }
}

impl TextInputDeclarations {
    fn record(&mut self, request: &zwp_text_input_v3::Request) {
        match request {
            zwp_text_input_v3::Request::Enable => {
                self.pending = PendingDeclaration {
                    enable: Some(true),
                    purpose: Some(None),
                };
            }
            zwp_text_input_v3::Request::Disable => self.pending.enable = Some(false),
            zwp_text_input_v3::Request::SetContentType { purpose, .. } => {
                self.pending.purpose = Some(purpose.into_result().ok());
            }
            zwp_text_input_v3::Request::Commit => {
                let pending = std::mem::take(&mut self.pending);
                self.current.apply(pending);
            }
            _ => {}
        }
    }
}

impl TextInputPurposes {
    pub fn record(&mut self, text_input: &ZwpTextInputV3, request: &zwp_text_input_v3::Request) {
        self.declarations
            .entry(text_input.id())
            .or_default()
            .record(request);
    }

    pub fn forget(&mut self, text_input: &ZwpTextInputV3) {
        self.declarations.remove(&text_input.id());
    }

    pub fn applied(&self, text_input: &ZwpTextInputV3) -> Option<AppliedTextInput> {
        self.declarations
            .get(&text_input.id())
            .map(|declarations| declarations.current)
    }
}

#[cfg(test)]
mod tests {
    use smithay::reexports::{
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{ContentHint, Request},
        wayland_server::WEnum,
    };

    use super::*;

    fn content_type(purpose: ContentPurpose) -> Request {
        Request::SetContentType {
            hint: WEnum::Value(ContentHint::empty()),
            purpose: WEnum::Value(purpose),
        }
    }

    fn after(requests: &[Request]) -> AppliedTextInput {
        let mut declarations = TextInputDeclarations::default();
        for request in requests {
            declarations.record(request);
        }
        declarations.current
    }

    #[test]
    fn nothing_applies_before_the_commit() {
        let applied = after(&[Request::Enable, content_type(ContentPurpose::Password)]);
        assert!(!applied.is_password());
        assert_eq!(applied.enables, 0);
    }

    #[test]
    fn an_enabled_password_field_is_known() {
        let applied = after(&[
            Request::Enable,
            content_type(ContentPurpose::Password),
            Request::Commit,
        ]);
        assert!(applied.is_password());
        assert_eq!(applied.enables, 1);
    }

    #[test]
    fn enable_resets_the_purpose() {
        let applied = after(&[
            Request::Enable,
            content_type(ContentPurpose::Password),
            Request::Commit,
            Request::Enable,
            Request::Commit,
        ]);
        assert!(applied.enabled);
        assert_eq!(applied.purpose, None);
        assert_eq!(applied.enables, 2);
    }

    #[test]
    fn disable_ends_the_field() {
        let applied = after(&[
            Request::Enable,
            content_type(ContentPurpose::Password),
            Request::Commit,
            Request::Disable,
            Request::Commit,
        ]);
        assert!(!applied.is_password());
    }

    #[test]
    fn a_later_commit_keeps_the_purpose_and_can_change_it() {
        let kept = after(&[
            Request::Enable,
            content_type(ContentPurpose::Password),
            Request::Commit,
            Request::Commit,
        ]);
        assert!(kept.is_password());
        assert_eq!(kept.enables, 1, "a commit without enable is the same field");
        let changed = after(&[
            Request::Enable,
            content_type(ContentPurpose::Password),
            Request::Commit,
            content_type(ContentPurpose::Terminal),
            Request::Commit,
        ]);
        assert!(changed.is_terminal());
    }
}
