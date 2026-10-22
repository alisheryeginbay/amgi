//! `amgi`: the collection from the terminal.

mod commands;
mod diff;
mod style;
mod locate;
mod text;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "amgi", version, about = "Plain-text flash cards, from the terminal")]
struct Cli {
    /// Collection folder. Defaults to the folder holding `.amgi.sqlite` at or above the
    /// current one, then to the folder last opened in the app.
    #[arg(long, short = 'C', global = true, env = "AMGI_DIR", value_name = "PATH")]
    dir: Option<PathBuf>,

    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// When to use colors.
    #[arg(long, global = true, value_enum, default_value_t, value_name = "WHEN")]
    color: style::ColorChoice,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cards left to study today, and the decks they're in.
    Status {
        /// One line, such as `12 due · 5 new`, or nothing when all caught up.
        #[arg(long, short)]
        short: bool,
    },
    /// Problems in the deck files, as `path:line: message`. Exits 1 if there are any.
    Check,
    /// Find cards with the app's search syntax, such as `deck:french is:due`.
    Search {
        /// Search words and filters; all must match. Empty lists every card.
        query: Vec<String>,
        /// Run a saved view, such as `Leeches`, combined with any query given.
        #[arg(long, short)]
        view: Option<String>,
        /// Print only card ids, one per line.
        #[arg(long, conflicts_with = "json")]
        ids: bool,
    },
    /// Every deck, with cards due, new and in total.
    Decks,
    /// Every tag, with how many cards have it.
    Tags,
    /// A card's schedule, memory and review history.
    Info {
        /// A card id, `path:line` (any line of the card), or a search matching one card.
        card: Vec<String>,
    },
    /// Files in the `media` folder that no card links to. Lists them; `--trash` moves
    /// them to the Trash.
    Prune {
        /// Move the unused files to the Trash.
        #[arg(long)]
        trash: bool,
    },
    /// Cards added, removed and changed since a git revision, `HEAD` by default, as
    /// the files on disk have them.
    Diff {
        /// Compare against this revision, or between two revisions.
        #[arg(num_args = 0..=2, value_name = "REV")]
        revs: Vec<String>,
        /// Compare the files staged for the next commit instead of those on disk.
        #[arg(long, visible_alias = "cached")]
        staged: bool,
    },
    /// Streak, retention, card counts, recent reviews and the coming days' workload.
    Stats {
        /// Days of review history to show.
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// Only this deck and its subdecks, such as `lang/french`.
        #[arg(long, short)]
        deck: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    style::init(cli.color);
    let collection = match locate::open(cli.dir.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {e}", style::red("amgi:"));
            return ExitCode::from(2);
        }
    };
    let out = commands::Output { json: cli.json };
    let result = match cli.command {
        Command::Status { short } => commands::status(&collection, out, short),
        Command::Check => commands::check(&collection, out),
        Command::Search { query, view, ids } => commands::search(&collection, out, &query.join(" "), view.as_deref(), ids),
        Command::Decks => commands::decks(&collection, out),
        Command::Tags => commands::tags(&collection, out),
        Command::Info { card } => commands::info(&collection, out, &card.join(" ")),
        Command::Prune { trash } => commands::prune(&collection, out, trash),
        Command::Diff { revs, staged } => commands::diff(&collection, out, &revs, staged),
        Command::Stats { days, deck } => commands::stats(&collection, out, days, deck.as_deref()),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e}", style::red("amgi:"));
            ExitCode::from(2)
        }
    }
}
