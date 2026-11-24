#![forbid(unsafe_code)]
mod adapter;
mod cli;
mod compat;
mod eval;
mod project;
mod routing;
mod run;
mod service;
mod store;
mod supervisor;
mod wire;
mod assets {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}

pub fn uid() -> String {
    hex::encode(rand::random::<[u8; 12]>())
}

fn main() -> std::process::ExitCode {
    match cli::execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
