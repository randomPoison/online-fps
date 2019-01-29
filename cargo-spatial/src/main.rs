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
    }
}
