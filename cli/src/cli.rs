//! Command grammar. Mirrors the illustrative CLI in PRD §10; not final.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "git story",
    bin_name = "git story",
    version,
    about = "Evidence-backed engineering memory for Git commits",
    args_conflicts_with_subcommands = true,
    after_help = "With no command, shows the story for <REV> (default HEAD).\n\nEvery command accepts --json and then prints one versioned envelope:\n  {\"schema\":\"gitstory.cli/v1\",\"command\":...,\"ok\":true|false,\"data\":...,\"error\":{\"code\":\"E_...\",\"message\":...}}"
)]
pub struct Cli {
    /// Emit the versioned JSON envelope instead of human output. Never prompts.
    #[arg(long, global = true)]
    pub json: bool,

    /// Compact agent view: summary, implemented decisions, rejected options,
    /// unknowns. No hypotheses or discarded claims.
    #[arg(long, global = true)]
    pub brief: bool,

    /// Commit to show when no command is given.
    pub rev: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum NoteType {
    Decision,
    Reason,
    Constraint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TestResult {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Markdown,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a .gitstory file: store location, collection and egress policy.
    Init,
    /// Record a decision, reason or constraint, or a test result.
    Note {
        /// What was decided and why.
        text: Option<String>,
        #[arg(long = "type", value_enum)]
        kind: Option<NoteType>,
        /// The story whose decision this changes.
        #[arg(long)]
        supersedes: Option<String>,
        /// Test command, recorded with --result.
        #[arg(long, requires = "result", conflicts_with = "text")]
        test: Option<String>,
        #[arg(long, value_enum, requires = "test")]
        result: Option<TestResult>,
    },
    /// Draft a story from the evidence collected so far.
    Draft {
        #[arg(long)]
        work_unit: Option<String>,
    },
    /// Reconcile a work unit's drafts against the commit that landed.
    Land {
        #[arg(long)]
        work_unit: String,
        #[arg(long)]
        to: String,
    },
    /// Stories behind the lines of a file, following supersession forward.
    Blame {
        /// `path` or `path:start-end`.
        target: String,
    },
    /// Every story that ever touched a path, current decisions first.
    Path { path: String },
    /// The supersession chain of a story, oldest first.
    History { story: String },
    /// Full-text search over summaries, claims, goals and paths.
    Search { query: String },
    /// Link stories, or accept or reject a proposed link.
    Link {
        story: Option<String>,
        #[arg(long)]
        supersedes: Option<String>,
        #[arg(long)]
        related: Option<String>,
        #[arg(long, conflicts_with_all = ["story", "reject"])]
        accept: Option<String>,
        #[arg(long, conflicts_with_all = ["story", "accept"])]
        reject: Option<String>,
    },
    /// Edit a story; the change is versioned and becomes stated evidence.
    Edit { rev: String },
    /// Export stories with their durable evidence, links and egress manifests.
    Export {
        #[arg(long, value_enum)]
        format: Format,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Import an export into this store.
    Import { dir: PathBuf },
}

impl Cli {
    /// Stable command name used in the JSON envelope.
    pub fn name(&self) -> &'static str {
        match &self.command {
            None => "show",
            Some(c) => match c {
                Command::Init => "init",
                Command::Note { .. } => "note",
                Command::Draft { .. } => "draft",
                Command::Land { .. } => "land",
                Command::Blame { .. } => "blame",
                Command::Path { .. } => "path",
                Command::History { .. } => "history",
                Command::Search { .. } => "search",
                Command::Link { .. } => "link",
                Command::Edit { .. } => "edit",
                Command::Export { .. } => "export",
                Command::Import { .. } => "import",
            },
        }
    }
}
