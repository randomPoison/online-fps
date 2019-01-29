use crate::opt::*;
use std::process;

/// Builds workers and then runs `spatial local launch`.
pub fn launch(opt: &Opt, local: &Local, launch: &LocalLaunch) {
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

    process::Command::new("spatial")
        .arg("local")
        .arg("launch")
        .arg("--launch_config=deployment.json")
        .status()
        .expect("Failed to run spatial");
}
