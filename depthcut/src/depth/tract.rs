use std::{path::Path, sync::Arc, thread};

use anyhow::{Context, Result, ensure};
use tract_linalg::multithread::{Executor, multithread_tract_scope};
use tract_onnx::prelude::*;

use super::{DepthModel, ModelInput};
use crate::Plane;

/// Hyper-threads share a core's vector units, which the matrix kernels keep
/// busy already: one thread a physical core runs fastest.
const THREADS_PER_CORE: usize = 2;

/// The network on tract's CPU runtime, which picks its AVX2, AVX-512 or VNNI
/// kernels at run time and spreads matrix products over the cores.
pub struct TractModel(Arc<TypedRunnableModel>);

impl DepthModel for TractModel {
    fn load(file: &Path, (width, height): (u32, u32)) -> Result<Self> {
        let shape = [1, 3, height as usize, width as usize];
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .model_for_path(file)?
            .with_input_fact(0, f32::fact(shape).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self(plan))
    }

    fn estimate(self, input: ModelInput) -> Result<Plane> {
        let (width, height) = input.dimensions();
        let shape = [1, 3, height as usize, width as usize];
        let tensor = Tensor::from_shape(&shape, &input.planar)?;
        drop(input);
        let outputs = multithread_tract_scope(Executor::multithread(threads()), || {
            self.0.run(tvec!(tensor.into()))
        })?;
        drop(self);
        let output = outputs.first().context("the model gave no output")?;
        let depth = output.try_as_plain_ram()?.as_slice::<f32>()?;
        let pixels = width as usize * height as usize;
        ensure!(
            depth.len() == pixels,
            "the model gave {} depths for {width} × {height} pixels",
            depth.len()
        );
        Plane::from_raw(width, height, depth.to_vec()).context("the depth plane has no pixels")
    }
}

fn threads() -> usize {
    thread::available_parallelism().map_or(1, |logical| (logical.get() / THREADS_PER_CORE).max(1))
}
