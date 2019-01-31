use crate::opt::*;
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
            Generate::EntityId => {
                println!("Entity ID: {}", generate_entity_id());
            }
        },
    }
}

/// Generates a random, valid Entity ID.
///
/// Entity IDs are `i64` values that must be:
///
/// * Greater than 100.
/// * Not in the range 190000 to 199999.
fn generate_entity_id() -> i64 {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    loop {
        let num = rng.gen();
        if num > 100 && (num < 190_000 || num > 199_999) {
            return num;
        }
    }
}
