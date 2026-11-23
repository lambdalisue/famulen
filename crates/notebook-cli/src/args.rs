//! What the command line takes.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "famulen-notebook",
    version,
    about = "Read and write a notebook by its rules"
)]
pub struct Cli {
    /// The notebook's directory.
    #[arg(
        long,
        short = 'C',
        env = "FAMULEN_NOTEBOOK",
        default_value = ".",
        global = true
    )]
    pub notebook: PathBuf,
    #[command(flatten)]
    pub sign: Sign,
    #[command(subcommand)]
    pub command: Command,
}

/// Who writes, and what with: the commit's author and trailers.
#[derive(Args)]
pub struct Sign {
    /// The author, `Name <email>`. Without it, Git's own user.
    #[arg(long, env = "FAMULEN_NOTEBOOK_AUTHOR", global = true)]
    pub author: Option<String>,
    /// Someone who wrote it with the author, `Name <email>`.
    #[arg(long = "co-author", global = true)]
    pub co_authors: Vec<String>,
    /// The application or agent an AI writer runs in, with its version.
    #[arg(long, env = "FAMULEN_NOTEBOOK_AGENT", global = true)]
    pub agent: Option<String>,
    /// The model of an AI writer.
    #[arg(long, env = "FAMULEN_NOTEBOOK_MODEL", global = true)]
    pub model: Option<String>,
    /// What the change was written from.
    #[arg(long = "source", global = true)]
    pub sources: Vec<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Make the notebook, or put back what is missing from it.
    Init,
    /// Tell where the notebook strays from its rules.
    Check {
        /// The files to check, relative to the notebook. Without them, all.
        paths: Vec<String>,
        /// Put right what there is one way to put right, and commit it.
        #[arg(long)]
        fix: bool,
        #[arg(long)]
        json: bool,
    },
    /// Search memories and records.
    Search(SearchArgs),
    /// Read the index again from the files.
    Index,
    /// Print a file of the notebook, given relative to it.
    Get { path: String },
    /// List the records of a kind with their codes, or the kinds when none is
    /// given.
    List {
        kind: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Add a record. Prints its id.
    Add {
        kind: String,
        #[arg(long)]
        title: String,
        /// Its notes.
        #[arg(long, default_value = "")]
        text: String,
        /// A field, `Name=value`, or metadata such as `labels=[a, b]`. A field
        /// of a section is `Section/Name=value`. A value in `[a, b]` is a
        /// list.
        #[arg(long = "field", value_parser = key_value)]
        fields: Vec<(String, String)>,
    },
    /// Change a record.
    Set {
        kind: String,
        /// Its id or code.
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "field", value_parser = key_value)]
        fields: Vec<(String, String)>,
        /// A field to remove.
        #[arg(long = "unset")]
        unset: Vec<String>,
    },
    /// Remove a record.
    Rm {
        kind: String,
        /// Its id or code.
        id: String,
    },
    /// Add a line to a record's log.
    Log {
        kind: String,
        /// Its id or code.
        id: String,
        text: String,
        /// Who did it.
        #[arg(long, default_value = "owner")]
        who: String,
    },
    /// Write down a memory: in core, pending or the day's episodic memory.
    Remember {
        /// core, pending or episodic.
        layer: String,
        text: String,
        /// An attribute, `key=value`.
        #[arg(long = "attr", value_parser = key_value)]
        attrs: Vec<(String, String)>,
    },
    /// Keep a long-term memory in a file of its own.
    Keep {
        /// semantic, procedural or prospective.
        group: String,
        /// The file name, without `.md`.
        name: String,
        #[arg(long)]
        summary: String,
        #[arg(long, default_value = "")]
        text: String,
        #[arg(long = "attr", value_parser = key_value)]
        attrs: Vec<(String, String)>,
    },
    /// Count one use of a memory.
    Use { id: String },
    /// Commit what was changed by hand, in the signer's name.
    Commit {
        #[arg(long, short = 'm', default_value = "手で書き換えた")]
        message: String,
        /// The files to commit, relative to the notebook. Without them,
        /// every change.
        paths: Vec<String>,
    },
}

#[derive(Args)]
pub struct SearchArgs {
    /// Words; every one must appear.
    pub words: Vec<String>,
    /// Where to look: memory, memory/long, records/tasks. Repeatable.
    #[arg(long = "in")]
    pub scopes: Vec<String>,
    #[arg(long)]
    pub workgroup: Option<String>,
    #[arg(long = "field", value_parser = key_value)]
    pub fields: Vec<(String, String)>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    /// Look in forgotten memory too.
    #[arg(long)]
    pub forgotten: bool,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    #[arg(long)]
    pub json: bool,
}

fn key_value(text: &str) -> Result<(String, String), String> {
    let (k, v) = text
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, not {text:?}"))?;
    Ok((k.trim().to_owned(), v.trim().to_owned()))
}
