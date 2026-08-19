use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub fn next() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                Self(NEXT.fetch_add(1, Ordering::Relaxed))
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }
    };
}

id_type!(
    /// Identifies one mapped toplevel for its whole lifetime.
    ///
    /// A newtype rather than a bare `u64` so a workspace index or a monitor
    /// number cannot be passed where a window was meant.
    WindowId
);

id_type!(
    /// Stable across reaping and reordering. The user-visible workspace *index*
    /// is positional and shifts as workspaces come and go; this does not, which
    /// is what makes rebasing after a `retain` safe.
    WorkspaceId
);

id_type!(
    /// A `Copy` handle for an `Output`, cached in its `UserDataMap`.
    OutputId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_monotonic() {
        let a = WindowId::next();
        let b = WindowId::next();
        assert_ne!(a, b);
        assert!(a < b);
    }

    #[test]
    fn id_types_have_independent_counters() {
        // Sharing a counter would be harmless but confusing in logs. Compared as
        // a delta rather than against 1, since tests share the process.
        let before = WorkspaceId::next().raw();
        for _ in 0..5 {
            let _ = WindowId::next();
        }
        assert_eq!(WorkspaceId::next().raw(), before + 1);
    }
}
