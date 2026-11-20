//! `move`: asks the compositor to move windows to an output by name.

use anyhow::bail;

use crate::placements;

/// Moves every window whose identifier or app id is `window`.
pub fn run(window: &str, output: &str) -> anyhow::Result<()> {
    let (mut queue, mut placements) = placements::load(2)?;
    let mut moved = 0;
    for (handle, placement) in &placements.windows {
        if placement.identifier == window || placement.app_id == window {
            handle.move_to_output(output.to_owned());
            moved += 1;
        }
    }
    if moved == 0 {
        bail!("no window is called {window}");
    }
    queue.roundtrip(&mut placements)?;
    println!("asked to move {moved} window(s) to {output}");
    Ok(())
}
