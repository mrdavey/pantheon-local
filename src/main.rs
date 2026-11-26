#![forbid(unsafe_code)]
mod activity;
mod adapter;
mod attachments;
mod chat_stream;
mod cli;
mod compat;
mod eval;
mod folder_picker;
mod launch;
mod metrics;
mod pricing;
mod project;
mod routing;
mod run;
mod service;
mod store;
mod supervisor;
#[cfg(test)]
#[path = "../tests/common/example.rs"]
mod test_examples;
mod turn_log;
mod update;
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
