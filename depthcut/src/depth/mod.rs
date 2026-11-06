//! Depth Anything V2 Small, behind [`DepthModel`] so another runtime, such as
//! a Vulkan one, can stand in for tract.

mod model_file;
mod tract;

use std::path::Path;

use anyhow::Result;
use image::{
    RgbImage,
    imageops::{self, FilterType},
};

pub use model_file::model_file;
pub use tract::TractModel;

use crate::{Plane, util::fit::patch_grid};

/// The network sees its input as square patches this many pixels wide.
const PATCH_SIDE: u32 = 14;
/// As many patches as the 518 × 518 square the network was trained at,
/// whatever the picture's shape: attention costs the square of the count.
const PATCH_BUDGET: u32 = 37 * 37;
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_DEVIATION: [f32; 3] = [0.229, 0.224, 0.225];

/// The picture as the network takes it: resized onto a whole patch grid, as
/// planar RGB normalised by the ImageNet statistics it was trained on.
pub struct ModelInput {
    width: u32,
    height: u32,
    planar: Vec<f32>,
}

impl ModelInput {
    pub fn new(picture: &RgbImage) -> Self {
        let (width, height) = patch_grid(picture.dimensions(), PATCH_SIDE, PATCH_BUDGET);
        let resized = imageops::resize(picture, width, height, FilterType::Triangle);
        let planar = (0..IMAGENET_MEAN.len())
            .flat_map(|channel| {
                resized.pixels().map(move |pixel| {
                    (f32::from(pixel[channel]) / 255.0 - IMAGENET_MEAN[channel])
                        / IMAGENET_DEVIATION[channel]
                })
            })
            .collect();
        Self {
            width,
            height,
            planar,
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// A depth network, loaded for one input size and run once.
pub trait DepthModel: Sized {
    fn load(file: &Path, dimensions: (u32, u32)) -> Result<Self>;

    /// Relative inverse depth, larger nearer, at the input's size. Takes the
    /// model, so its memory goes as soon as the answer is in.
    fn estimate(self, input: ModelInput) -> Result<Plane>;
}
