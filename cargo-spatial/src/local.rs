use crate::opt::*;
use std::ffi::OsString;
use std::process;
use tap::*;

/// Builds workers and then runs `spatial local launch`.
pub fn launch(_opt: &Opt, _local: &Local, launch: &LocalLaunch) {
    // Use `cargo install` to build workers and copy the exectuables to the build directory.
    if !launch.no_build {
        process::Command::new("cargo")
            .args(&["install", "--root", "./build/debug", "--debug", "--force"])
            .arg("--path")
            .arg("./workers")
            .arg("--bin")
            .arg("server")
            .status()
            .expect("Failed to build worker bin");
    }

    // Run `spatial alpha local launch` with any user-specified flags.
    let mut command = process::Command::new("spatial");
    command.args(&["alpha", "local", "launch"]);
    if let Some(launch_config) = &launch.launch_config {
        let arg = OsString::from("--launch_config=").tap(|arg| arg.push(launch_config));
        command.arg(arg);
    }
    command.status().expect("Failed to run spatial");
}
