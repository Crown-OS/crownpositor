/// Every bound a scene is held to, and the protocol's payload caps.
pub struct Limits;

impl Limits {
    pub const SCENE_BYTES: usize = 256 * 1024;
    pub const SLOT_BYTES: usize = crownplugin_view::MAX_TREE_BYTES;
    pub const ASSET_BYTES: usize = 32 * 1024 * 1024;
    pub const ASSETS_TOTAL_BYTES: usize = 64 * 1024 * 1024;
    /// Decoded size of one image, so a small file cannot inflate into gigabytes.
    pub const IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
    pub const SLOTS: usize = 16;
    pub const SLOT_UPDATES_PER_SECOND: u32 = 30;
    pub const NODES: usize = 1024;
    pub const DEPTH: usize = 24;
    pub const TEXT_CHARS: usize = 512;
    pub const ID_CHARS: usize = 160;
    pub const FORMAT_CHARS: usize = 64;
    pub const FONT_CHARS: usize = 128;
    pub const PATH_CHARS: usize = 4096;
    pub const LENGTH: f32 = 8192.0;
    pub const OFFSET: f32 = 4096.0;
    pub const EFFECTS: usize = 8;
    pub const DURATION_MS: u32 = 2000;
    pub const BLUR_RADIUS: f32 = 64.0;
    pub const SLIDE_DISTANCE: f32 = 600.0;
    pub const ZOOM: std::ops::RangeInclusive<f32> = 0.5..=1.5;
    pub const PROMPT_TIMEOUT_SECS: u32 = 600;
}
