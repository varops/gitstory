//! `git-story`: Git discovers this binary as the `git story` subcommand.

mod adapters;
mod cli;
mod cmd;
mod collect;
mod config;
mod error;
mod ghf;
mod git;
mod hooks;
mod index;
mod model;
mod render;
mod s3;
mod scrub;
mod store;
mod synth;
mod workspace;

use std::io::IsTerminal;
use std::process;

use clap::Parser;
use clap::error::ErrorKind;
use gitstory_proto::{ErrorCode, ExitCode, Response};

use cli::{Cli, Command, NoteType, TestResult};
use cmd::{EditArgs, Env, Out};
use error::Result;

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
    let json = cli.json;
    let env = Env {
        cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        brief: cli.brief,
    };
    match dispatch(&env, cli) {
        Ok(out) => {
            if json {
                emit_json(&Response::ok(name, out.data));
            } else if !out.human.is_empty() {
                println!("{}", out.human.trim_end());
            }
        }
        Err(f) => {
            if json {
                emit_json(&Response::err(name, f.code, &f.message, f.data));
            } else {
                eprintln!("git story {name}: {} ({})", f.message, f.code);
            }
            process::exit(f.code.exit_code() as i32);
        }
    }
}

fn dispatch(env: &Env, cli: Cli) -> Result<Out> {
    let Some(command) = cli.command else {
        return cmd::show(env, cli.rev);
    };
    match command {
        Command::Init {
            repository,
            trust,
            force,
        } => cmd::init(env, repository, trust, force),
        Command::Trust { endpoint } => cmd::trust(env, endpoint),
        Command::Show { rev } => cmd::show(env, rev),
        Command::Note {
            text,
            kind,
            supersedes,
            test,
            result,
            work_unit,
        } => {
            let kind = kind.map(|k| match k {
                NoteType::Decision => "decision",
                NoteType::Reason => "reason",
                NoteType::Constraint => "constraint",
            });
            let test = test.map(|t| (t, result == Some(TestResult::Pass)));
            cmd::note(env, text, kind, supersedes, test, work_unit)
        }
        Command::Draft {
            work_unit,
            background,
        } => cmd::draft(env, work_unit, background),
        Command::Land { work_unit, to } => cmd::land(env, work_unit, to),
        Command::Blame { target } => cmd::blame(env, target),
        Command::Path { path } => cmd::path(env, path),
        Command::History { story } => cmd::history(env, story),
        Command::Search { query } => cmd::search(env, query),
        Command::Validate { rev } => cmd::validate(env, rev),
        Command::Link {
            story,
            supersedes,
            related,
            remove,
            accept,
            reject,
        } => cmd::link(env, story, supersedes, related, accept, reject, remove),
        Command::Proposals => cmd::proposals(env),
        Command::Edit {
            rev,
            story,
            summary,
            add_decision,
            add_unknown,
            promote,
            demote,
            review,
        } => cmd::edit(
            env,
            EditArgs {
                rev,
                story,
                summary,
                add_decision,
                add_unknown,
                promote,
                demote,
                review,
            },
        ),
        Command::Export { format, out } => cmd::export(
            env,
            if format == cli::Format::Markdown {
                "markdown"
            } else {
                "json"
            },
            out,
        ),
        Command::Import { dir } => cmd::import(env, dir),
        Command::Status => cmd::status(env),
        Command::Gc { dry_run } => cmd::gc(env, dry_run),
        Command::Hooks {
            action,
            harness,
            scope,
        } => cmd::hooks(env, &action, harness, scope),
        Command::Skill { action, scope } => cmd::skill(env, &action, scope),
    }
}

/// Human output goes to a terminal; anything else gets plain text too, but
/// commands never prompt either way.
#[allow(dead_code)]
fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

fn emit_json(resp: &Response) {
    println!(
        "{}",
        serde_json::to_string(resp).expect("response serializes")
    );
}
