//! `crown-depthcut <image>`: cuts the subject out of the wallpaper at
//! `image` into the mask [`depthcut::mask_path`] names, then exits, which
//! frees every byte the model took.

mod depth;
mod mask;
mod util;
mod wallpaper;

use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Luma, codecs::png::PngEncoder};

use depth::{DepthModel, ModelInput, TractModel};

type Plane = ImageBuffer<Luma<f32>, Vec<f32>>;

fn main() -> Result<()> {
    let image = image_argument()?;
    let mask_file = depthcut::mask_path(&image)?;
    let model_file = depth::model_file()?;
    let picture = wallpaper::load(&image, mask::LONG_SIDE)
        .with_context(|| format!("cannot decode {}", image.display()))?;
    let guide = mask::guide(&picture);
    let input = ModelInput::new(&picture);
    drop(picture);
    let disparity = TractModel::load(&model_file, input.dimensions())
        .with_context(|| format!("cannot load {}", model_file.display()))?
        .estimate(input)
        .context("the depth model failed")?;
    let mask = mask::cut(disparity, &guide);
    let mut png = Vec::new();
    mask.write_with_encoder(PngEncoder::new(&mut png))?;
    util::atomic_write::write_atomically(&mask_file, &png)
        .with_context(|| format!("cannot write {}", mask_file.display()))
}

fn image_argument() -> Result<PathBuf> {
    let mut arguments = env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (Some(image), None) => Ok(image.into()),
        _ => bail!("usage: {} <image>", depthcut::HELPER_NAME),
    }
}
