use crate::opt::*;
use std::ffi::OsString;
use std::process;
use tap::*;

/// Builds workers and then runs `spatial local launch`.
pub fn launch(_opt: &Opt, _local: &Local, launch: &LocalLaunch) {
    process::Command::new("which")
        .arg("setup")
        .status()
        .unwrap();

    // Run codegen and such.
    // TODO: Use the spatialos-sdk-tools crate directly rather than invoking the CLI.
    // TODO: Make the various flags configurable.
    let status = dbg!(process::Command::new("setup").args(&[
        "-s",
        "schema",
        "-c",
        "workers/src/generated.rs",
        "-o",
        "schema/bin",
    ]))
    .status()
    .expect("Failed to run setup script");

    if !status.success() {
        return;
    }

    // Use `cargo install` to build workers and copy the exectuables to the build directory.
    if !launch.no_build {
        let status = process::Command::new("cargo")
            .args(&["install", "--root", "./build/debug", "--debug", "--force"])
            // TODO: These values need to be configurable.
            .arg("--path")
            .arg("./workers")
            .arg("--bin")
            .arg("server")
            .status()
            .expect("Failed to build worker bin");

        if !status.success() {
            return;
        }
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
