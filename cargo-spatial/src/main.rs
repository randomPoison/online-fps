use crate::opt::*;
use serde::*;
use structopt::StructOpt;

mod local;
mod opt;

fn main() {
    let opt = Opt::from_args();
    match &opt.command {
        Command::Local(local) => match local {
            Local::Launch(launch) => local::launch(&opt, local, launch),
        },

        Command::Generate { command } => match command {
            Generate::ComponentId => {
                println!("Component ID: {}", generate_component_id());
            }
        },
    }
}

/// Generates a random, valid component ID.
///
/// Component IDs are `i32` values that must be:
///
/// * Greater than 100.
/// * Less than 536,870,911.
/// * Not in the range 190,000 to 199999.
fn generate_component_id() -> i32 {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    loop {
        let num = rng.gen();
        if num > 100 && (num < 190_000 || num > 199_999) && num < 536_870_911 {
            return num;
        }
    }
}

/// Project configuration stored in the `Cargo.toml` file at project's root.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    /// The list of worker projects to be built.
    ///
    /// If empty, the root project is assumed to contain all workers.
    workers: Vec<String>,

    /// The file to use as output for code generation.
    ///
    /// Defaults to `src/generated.rs`.
    codegen_out: Option<String>,

    /// The directory containing schema files for the project.
    ///
    /// Defaults to `./schema`.
    schema_dir: Option<String>,

    /// The directory where built workers are put.
    ///
    /// Defaults to `./build`.
    build_dir: Option<String>,
}
