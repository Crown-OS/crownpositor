use std::process::ExitCode;

fn main() -> anyhow::Result<ExitCode> {
    Ok(compositor::run()?.status())
}
