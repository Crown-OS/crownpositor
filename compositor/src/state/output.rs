use std::{collections::HashMap, time::Duration};

use smithay::output::Output;

pub struct OutputConfig {}

impl OutputConfig {
    pub fn new() -> Self {
        OutputConfig {}
    }
}

pub struct OutputState {
    pub outputs: HashMap<Output, OutputConfig>,
}

impl OutputState {
    pub fn new() -> Self {
        OutputState {
            outputs: HashMap::default(),
        }
    }

    pub fn create_output(&mut self, output: Output, refresh_interval: Option<Duration>) {
        self.outputs.insert(output, OutputConfig::new());
    }
    pub fn remove_output(&self) {
        // TODO: Implement method
    }
}
