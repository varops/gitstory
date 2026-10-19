//! Command grammar (PRD §10).

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::hooks::{Harness, Scope};

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
    Init {
        /// Repository id in the store (default: from the origin remote).
        #[arg(long)]
        repository: Option<String>,
        /// Trust the configured model endpoint now.
        #[arg(long)]
        trust: bool,
        /// Overwrite an existing .gitstory.
        #[arg(long)]
        force: bool,
    },
    /// Trust a model endpoint (default: the configured ones).
    Trust { endpoint: Option<String> },
    /// Show the story for a commit or story id.
    Show { rev: Option<String> },
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
        #[arg(long)]
        work_unit: Option<String>,
    },
    /// Draft a story from the evidence collected so far. Drafts stay local.
    Draft {
        #[arg(long)]
        work_unit: Option<String>,
        /// Return at once and draft in a detached process (for hooks).
        #[arg(long)]
        background: bool,
    },
    /// Reconcile a work unit's drafts against the commit that landed, and
    /// write the story to the shared store.
    Land {
        #[arg(long)]
        work_unit: Option<String>,
        /// The landed commit, reachable from the main branch.
        #[arg(long, default_value = "HEAD")]
        to: String,
        /// The work landed as the commits after REV up to --to, pushed
        /// straight to main.
        #[arg(long)]
        since: Option<String>,
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
    /// Re-check a landed story's claims against its stored evidence only.
    Validate {
        #[arg(default_value = "HEAD")]
        rev: String,
    },
    /// Link stories, or accept or reject a proposed link.
    Link {
        story: Option<String>,
        #[arg(long, conflicts_with = "related")]
        supersedes: Option<String>,
        #[arg(long)]
        related: Option<String>,
        /// Remove the link instead of adding it.
        #[arg(long)]
        remove: bool,
        #[arg(long, conflicts_with_all = ["story", "reject"])]
        accept: Option<String>,
        #[arg(long, conflicts_with_all = ["story", "accept"])]
        reject: Option<String>,
    },
    /// Open link proposals.
    Proposals,
    /// Edit a landed story; the change is versioned and becomes stated evidence.
    Edit {
        /// Commit or story id.
        #[arg(default_value = "HEAD")]
        rev: String,
        /// The story, when the commit has several.
        #[arg(long)]
        story: Option<String>,
        #[arg(long)]
        summary: Option<String>,
        /// Add a stated decision.
        #[arg(long = "add-decision")]
        add_decision: Vec<String>,
        #[arg(long = "add-unknown")]
        add_unknown: Vec<String>,
        /// Promote hypothesis N (0-based) to a stated decision.
        #[arg(long)]
        promote: Vec<usize>,
        /// Demote decision N (0-based) to a hypothesis.
        #[arg(long)]
        demote: Vec<usize>,
        /// reviewed, flagged or unreviewed.
        #[arg(long)]
        review: Option<String>,
    },
    /// Export stories with their durable evidence, links and egress manifests.
    Export {
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Import an export into this store.
    Import { dir: PathBuf },
    /// Work units, drafts, store and egress policy.
    Status,
    /// Remove local collection for landed and expired work units.
    Gc {
        #[arg(long)]
        dry_run: bool,
    },
    /// Install, remove or inspect background drafting hooks.
    Hooks {
        #[arg(value_parser = ["install", "uninstall", "status"])]
        action: String,
        #[arg(value_enum)]
        harness: Harness,
        #[arg(long, value_enum, default_value = "project")]
        scope: Scope,
    },
    /// Install or print the agent skill.
    Skill {
        #[arg(value_parser = ["install", "show"])]
        action: String,
        #[arg(long, value_enum, default_value = "project")]
        scope: Scope,
    },
}

impl Cli {
    /// Stable command name used in the JSON envelope.
    pub fn name(&self) -> &'static str {
        match &self.command {
            None => "show",
            Some(c) => match c {
                Command::Init { .. } => "init",
                Command::Trust { .. } => "trust",
                Command::Show { .. } => "show",
                Command::Note { .. } => "note",
                Command::Draft { .. } => "draft",
                Command::Land { .. } => "land",
                Command::Blame { .. } => "blame",
                Command::Path { .. } => "path",
                Command::History { .. } => "history",
                Command::Search { .. } => "search",
                Command::Validate { .. } => "validate",
                Command::Link { .. } => "link",
                Command::Proposals => "proposals",
                Command::Edit { .. } => "edit",
                Command::Export { .. } => "export",
                Command::Import { .. } => "import",
                Command::Status => "status",
                Command::Gc { .. } => "gc",
                Command::Hooks { .. } => "hooks",
                Command::Skill { .. } => "skill",
            },
        }
    }
}
