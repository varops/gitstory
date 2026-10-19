//! `git-story`: Git discovers this binary as the `git story` subcommand.

mod cli;

use std::process;

use clap::Parser;
use clap::error::ErrorKind;
use gitstory_proto::{ErrorCode, ExitCode, Response};

use cli::Cli;

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let wants_json = std::env::args().any(|a| a == "--json");
            let is_info = matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            if wants_json && !is_info {
                let msg = err.render().to_string();
                emit_json(&Response::err("", ErrorCode::Usage, msg.trim(), None));
                process::exit(ExitCode::Usage as i32);
            }
            err.exit();
        }
    };

    let name = cli.name();
    let code = ErrorCode::NotImplemented;
    let message = format!("`{name}` is not implemented yet");
    if cli.json {
        emit_json(&Response::err(name, code, &message, None));
    } else {
        eprintln!("git story {name}: {message} ({code})");
    }
    process::exit(code.exit_code() as i32);
}

fn emit_json(resp: &Response) {
    println!(
        "{}",
        serde_json::to_string(resp).expect("response serializes")
    );
}
