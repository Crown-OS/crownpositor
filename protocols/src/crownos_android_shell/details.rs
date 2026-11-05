//! What an Android task window says about itself, and how it is
//! double-buffered: requests edit a pending copy, a commit makes it current.

use std::num::NonZeroU32;

pub use crownos_protocols::android_shell::v1::server::crownos_android_toplevel_v1::ResizeLock;

/// The Android task behind a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidTask {
    /// Android's task id, stable for the life of the task.
    pub id: u32,
    /// The Android package name.
    pub package: String,
}

/// A width to height ratio, both sides non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AspectRatio {
    pub width: NonZeroU32,
    pub height: NonZeroU32,
}

impl AspectRatio {
    /// `None` when either side is zero, which is how the protocol clears it.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            width: NonZeroU32::new(width)?,
            height: NonZeroU32::new(height)?,
        })
    }
}

/// An app's primary colour, `0xAARRGGBB` with a non-zero alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleColor(u32);

impl TitleColor {
    /// `None` for a zero alpha: the app has no colour of its own.
    pub fn new(argb: u32) -> Option<Self> {
        (argb >> 24 != 0).then_some(Self(argb))
    }

    pub fn argb(self) -> u32 {
        self.0
    }

    /// Straight (not premultiplied) red, green, blue and alpha in `0..=1`.
    pub fn rgba(self) -> [f32; 4] {
        let channel = |shift: u32| f32::from((self.0 >> shift) as u8) / f32::from(u8::MAX);
        [channel(16), channel(8), channel(0), channel(24)]
    }
}

/// How a window may be resized, with the ratio already resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeLock {
    #[default]
    Free,
    /// The app cannot resize: the window keeps the size it asks for.
    Fixed,
    /// Every resize keeps this ratio.
    Aspect(AspectRatio),
}

/// The committed Android details of one window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidDetails {
    pub task: Option<AndroidTask>,
    pub resize_lock: ResizeLock,
    pub aspect: Option<AspectRatio>,
    pub title_color: Option<TitleColor>,
}

impl Default for AndroidDetails {
    fn default() -> Self {
        Self {
            task: None,
            resize_lock: ResizeLock::None,
            aspect: None,
            title_color: None,
        }
    }
}

impl AndroidDetails {
    /// The lock as it applies: an aspect lock without a ratio resizes freely.
    pub fn size_lock(&self) -> SizeLock {
        match (self.resize_lock, self.aspect) {
            (ResizeLock::Fixed, _) => SizeLock::Fixed,
            (ResizeLock::Aspect, Some(ratio)) => SizeLock::Aspect(ratio),
            _ => SizeLock::Free,
        }
    }
}

/// The current details and whatever requests changed since the last commit.
#[derive(Debug, Default)]
pub(super) struct BufferedDetails {
    current: AndroidDetails,
    pending: Option<AndroidDetails>,
}

impl BufferedDetails {
    pub(super) fn current(&self) -> &AndroidDetails {
        &self.current
    }

    /// The copy requests edit, started from the current details.
    pub(super) fn pending(&mut self) -> &mut AndroidDetails {
        self.pending.get_or_insert_with(|| self.current.clone())
    }

    /// Applies what is pending. Returns whether the current details changed.
    pub(super) fn commit(&mut self) -> bool {
        match self.pending.take() {
            Some(next) if next != self.current => {
                self.current = next;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_wait_for_the_commit() {
        let mut details = BufferedDetails::default();
        details.pending().task = Some(AndroidTask {
            id: 7,
            package: "org.example".into(),
        });
        assert_eq!(details.current().task, None);

        assert!(details.commit());
        assert_eq!(details.current().task.as_ref().map(|task| task.id), Some(7));
    }

    #[test]
    fn a_commit_without_changes_reports_none() {
        let mut details = BufferedDetails::default();
        assert!(!details.commit());
        details.pending().resize_lock = ResizeLock::None;
        assert!(!details.commit(), "the same value again is no change");
    }

    #[test]
    fn a_zero_side_clears_the_ratio() {
        assert_eq!(AspectRatio::new(0, 9), None);
        assert_eq!(AspectRatio::new(16, 0), None);
        assert!(AspectRatio::new(16, 9).is_some());
    }

    #[test]
    fn an_aspect_lock_needs_a_ratio() {
        let mut details = AndroidDetails {
            resize_lock: ResizeLock::Aspect,
            ..AndroidDetails::default()
        };
        assert_eq!(details.size_lock(), SizeLock::Free);
        details.aspect = AspectRatio::new(9, 16);
        assert!(matches!(details.size_lock(), SizeLock::Aspect(_)));
        details.resize_lock = ResizeLock::Fixed;
        assert_eq!(details.size_lock(), SizeLock::Fixed);
    }

    #[test]
    fn a_transparent_title_color_is_no_color() {
        assert_eq!(TitleColor::new(0x00ff_0000), None);
        let red = TitleColor::new(0xffff_0000).map(TitleColor::rgba);
        assert_eq!(red, Some([1.0, 0.0, 0.0, 1.0]));
    }
}
