use crate::opt::*;
use crate::Config;
use cargo_metadata::*;
use std::ffi::OsString;
use std::path::*;
use std::process;
use tap::*;

/// Builds workers and then runs `spatial local launch`.
pub fn launch(_opt: &Opt, _local: &Local, launch: &LocalLaunch) {
    // Run `cargo metadata` to get the metadata for all packages in the workspace.
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("Failed to get cargo metadata");

    // Find the package corresponding to the root of the workspace.
    let manifest_path = metadata.workspace_root.join("Cargo.toml");
    let package = metadata
        .packages
        .iter()
        .find(|package| package.manifest_path == manifest_path)
        .expect("No root package found???");

    // Get configuration info from the crate metadata.
    let config: Config = package
        .metadata
        .get("spatialos")
        .and_then(|val| serde_json::from_value(val.clone()).ok())
        .unwrap_or_default();

    // Run codegen and such.
    // TODO: Use the spatialos-sdk-tools crate directly rather than invoking the CLI.
    let schema_dir = config.schema_dir.unwrap_or_else(|| "./schema".into());
    let codegen_out = PathBuf::from(&schema_dir).join("bin");
    let generated_file = config
        .codegen_out
        .unwrap_or_else(|| "src/generated.rs".into());
    let status = process::Command::new("setup")
        .arg("-s")
        .arg(&schema_dir)
        .arg("-c")
        .arg(&generated_file)
        .arg("-o")
        .arg(&codegen_out)
        .status()
        .expect("Failed to run setup script");

    if !status.success() {
        return;
    }

    // Use `cargo install` to build workers and copy the exectuables to the build
    // directory.
    //
    // TODO: Manually copy the built executables instead of using `cargo install`.
    // `cargo install` doesn't use the same build cache as normal builds, so it will
    // sometimes result in unnecessary recompilation, which can slow down launch times.
    if !launch.no_build {
        let build_dir = config.build_dir.unwrap_or_else(|| "./build".into());
        let build_dir = PathBuf::from(build_dir).join("debug");
        for worker_path in config.workers {
            let status = process::Command::new("cargo")
                .arg("install")
                .arg("--root")
                .arg(&build_dir)
                .arg("--debug")
                .arg("--force")
                .arg("--path")
                .arg(&worker_path)
                .status()
                .expect("Failed to build worker bin");

            if !status.success() {
                return;
            }
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
