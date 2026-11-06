//! The helper end to end on the installed model. Slow unoptimised, so run it
//! with `cargo test --release -p depthcut --features helper -- --ignored`.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{self, Command},
};

use image::{GrayImage, Rgb, RgbImage};

const MODEL: &str = "crownos/models/depth-anything-v2-small-int8.onnx";
const SCENE: (u32, u32) = (960, 540);

fn installed_model() -> Option<PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data_home
        .into_iter()
        .chain([PathBuf::from("/usr/share")])
        .map(|dir| dir.join(MODEL))
        .find(|model| model.is_file())
}

/// A pale sky over a dark hill rising from the bottom edge.
fn hill_scene() -> RgbImage {
    let (width, height) = SCENE;
    RgbImage::from_fn(width, height, |x, y| {
        let across = x as f32 / width as f32 - 0.5;
        let ridge = height as f32 * (0.45 + 1.2 * across * across);
        if (y as f32) < ridge {
            let fade = (y as f32 / ridge * 60.0) as u8;
            Rgb([120 + fade, 170 + fade / 2, 235])
        } else {
            Rgb([40, 70 + (y % 7) as u8 * 4, 35])
        }
    })
}

fn only_mask(directory: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let entries: Vec<PathBuf> = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    match entries.as_slice() {
        [mask] if mask.extension().is_some_and(|extension| extension == "png") => Ok(mask.clone()),
        other => Err(format!("expected one mask, found {other:?}").into()),
    }
}

#[test]
#[ignore = "needs the installed depth model, and an optimised build to be quick"]
fn the_helper_cuts_the_hill_out_of_the_sky() -> Result<(), Box<dyn Error>> {
    if installed_model().is_none() {
        eprintln!("skipped: no depth model installed");
        return Ok(());
    }
    let root = env::temp_dir().join(format!("depthcut-real-model-{}", process::id()));
    fs::create_dir_all(&root)?;
    let picture = root.join("hill.png");
    hill_scene().save(&picture)?;

    let cache = root.join("cache");
    let output = Command::new(env!("CARGO_BIN_EXE_crown-depthcut"))
        .arg(&picture)
        .env("XDG_CACHE_HOME", &cache)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mask: GrayImage = image::open(only_mask(&cache.join("crownos/depth"))?)?.into_luma8();
    assert_eq!(mask.dimensions(), SCENE);
    let (width, height) = SCENE;
    assert!(mask.get_pixel(width / 2, height - 10).0[0] > 200);
    assert!(mask.get_pixel(width / 2, 10).0[0] < 50);
    fs::remove_dir_all(root)?;
    Ok(())
}
