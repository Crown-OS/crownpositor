//! What one output info object was last told, and which of its events a new
//! snapshot has to send again.

use crownos_protocols::output_info::v1::server::crownos_output_info_v1::{
    Capability, CrownosOutputInfoV1, Shell, State,
};

/// Everything `crownos_output_info_v1` reports about one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    pub identity: String,
    pub shell: Shell,
    pub state: State,
    pub capabilities: Capability,
    pub scanout_depth: u32,
}

/// One event of `crownos_output_info_v1` that carries a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Identity,
    Shell,
    State,
    Capabilities,
    ScanoutDepth,
}

impl Field {
    const ALL: [Self; 5] = [
        Self::Identity,
        Self::Shell,
        Self::State,
        Self::Capabilities,
        Self::ScanoutDepth,
    ];
}

impl OutputInfo {
    /// The fields that differ from `previous`; every field when nothing was
    /// sent yet.
    pub fn changed_since<'a>(
        &'a self,
        previous: Option<&'a Self>,
    ) -> impl Iterator<Item = Field> + 'a {
        Field::ALL
            .into_iter()
            .filter(move |field| previous.is_none_or(|previous| !self.agrees_on(previous, *field)))
    }

    fn agrees_on(&self, other: &Self, field: Field) -> bool {
        match field {
            Field::Identity => self.identity == other.identity,
            Field::Shell => self.shell == other.shell,
            Field::State => self.state == other.state,
            Field::Capabilities => self.capabilities == other.capabilities,
            Field::ScanoutDepth => self.scanout_depth == other.scanout_depth,
        }
    }

    /// Sends the fields that differ from `previous`, then `done`. Returns
    /// whether anything was sent.
    pub(super) fn send(&self, object: &CrownosOutputInfoV1, previous: Option<&Self>) -> bool {
        let mut sent = false;
        for field in self.changed_since(previous) {
            sent = true;
            match field {
                Field::Identity => object.identity(self.identity.clone()),
                Field::Shell => object.shell(self.shell),
                Field::State => object.state(self.state),
                Field::Capabilities => object.capabilities(self.capabilities),
                Field::ScanoutDepth => object.scanout_depth(self.scanout_depth),
            }
        }
        if sent {
            object.done();
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> OutputInfo {
        OutputInfo {
            identity: "Dell U2720Q A1".to_owned(),
            shell: Shell::Bar | Shell::Dock,
            state: State::Primary,
            capabilities: Capability::empty(),
            scanout_depth: 8,
        }
    }

    #[test]
    fn a_first_snapshot_sends_every_field() {
        let current = info();
        assert_eq!(current.changed_since(None).collect::<Vec<_>>(), Field::ALL);
    }

    #[test]
    fn an_unchanged_snapshot_sends_nothing() {
        let current = info();
        assert_eq!(current.changed_since(Some(&info())).count(), 0);
    }

    #[test]
    fn only_the_fields_that_changed_are_sent() {
        let previous = info();
        let current = OutputInfo {
            identity: "DP-1".to_owned(),
            state: State::Primary | State::Focused,
            ..info()
        };
        assert_eq!(
            current.changed_since(Some(&previous)).collect::<Vec<_>>(),
            [Field::Identity, Field::State]
        );
    }

    #[test]
    fn a_new_depth_or_capability_is_sent_alone() {
        let previous = info();
        let deeper = OutputInfo {
            scanout_depth: 10,
            ..info()
        };
        let hdr = OutputInfo {
            capabilities: Capability::Hdr,
            ..info()
        };
        assert_eq!(
            deeper.changed_since(Some(&previous)).collect::<Vec<_>>(),
            [Field::ScanoutDepth]
        );
        assert_eq!(
            hdr.changed_since(Some(&previous)).collect::<Vec<_>>(),
            [Field::Capabilities]
        );
    }
}
