use std::path::PathBuf;
use structopt::StructOpt;

/// Build, run, and deploy SpatialOS workers written in Rust using Cargo.
#[derive(StructOpt)]
#[structopt(name = "cargo-spatial", rename_all = "kebab-case")]
pub struct Opt {
    /// Print output in JSON format
    ///
    /// Useful when you need to parse the Spatial CLI output.
    #[structopt(long, short)]
    pub json_output: bool,

    /// Disable dynamic output elements such as the spinner, progress bars, etc.
    #[structopt(long, short)]
    pub no_animation: bool,

    /// Sets the directory log files will be created in
    ///
    /// If not specified, this is set to <project_root>/logs when inside a project
    /// directory and logging is disabled when outside a project directory.
    #[structopt(parse(from_os_str))]
    pub log_directory: Option<PathBuf>,

    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(StructOpt)]
#[structopt(rename_all = "kebab-case")]
pub enum Command {
    /// Commands for developing and running a local SpatialOS project.
    #[structopt(name = "local")]
    Local(Local),
}

#[derive(StructOpt)]
#[structopt(rename_all = "kebab-case")]
pub enum Local {
    /// Start a SpatialOS simulation locally. Automatically builds workers.
    #[structopt(name = "launch")]
    Launch(LocalLaunch),
}

#[derive(StructOpt)]
#[structopt(rename_all = "kebab-case")]
pub struct LocalLaunch {
    /// Don't build workers before launching the local deployment.
    #[structopt(long, short)]
    pub no_build: bool,

    #[structopt(long, short = "c")]
    pub launch_config: Option<PathBuf>,
}
