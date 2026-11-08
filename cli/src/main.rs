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
    /// Problems in the deck files and `amgi.toml`, as `path:line: message`. Exits 1 if
    /// there are any.
    #[command(after_long_help = SETTINGS)]
    Check,
    /// Deck files that break the rules in `amgi.toml`, such as media links that don't
    /// start at the collection root, as `path:line:column: message`. Exits 1 if any are
    /// left.
    Lint {
        /// Fix what can be fixed, in the files.
        #[arg(long)]
        fix: bool,
    },
    /// Rewrite deck files in one layout: no trailing spaces, cards a blank line apart,
    /// tags a space apart. Cards, ids and history stay the same.
    Fmt {
        /// List the files that would change instead, and exit 1 if there are any.
        #[arg(long)]
        check: bool,
    },
    /// Find cards, with searches such as `deck:french is:due` or `#verbs lapses>3`.
    #[command(after_long_help = SEARCH_SYNTAX)]
    Search {
        /// Search words and filters; all must match. Empty lists every card.
        query: Vec<String>,
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
        /// Exit 1 if cards with review history are removed, as when a question is edited:
        /// the history stays in the database, but no card uses it.
        #[arg(long)]
        check: bool,
        /// Show only removed cards with review history.
        #[arg(long)]
        reviewed: bool,
    },
    /// Streak, time, retention, card counts, recent reviews and the coming days' workload.
    Stats {
        /// Days of review history to show.
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// Only this deck and its subdecks, such as `lang/french`.
        #[arg(long, short)]
        deck: Option<String>,
        /// Show the time spent each day rather than reviews.
        #[arg(long)]
        time: bool,
    },
    /// Reviews to expect in the coming days, simulated from each card's memory, and
    /// what studying new cards today, or every day, would add to them.
    Forecast {
        /// New cards to study today.
        #[arg(long, short, default_value_t = 0, value_name = "N")]
        new: u32,
        /// New cards to study each day from today, until the cards not yet studied run out.
        #[arg(long, value_name = "N", conflicts_with = "new")]
        per_day: Option<u32>,
        /// With --per-day, as if you add new cards as fast as you study them, so they
        /// never run out.
        #[arg(long, requires = "per_day")]
        keep_adding: bool,
        /// Days ahead to show; past a month, a line per week with
        /// the time a day on average.
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// Only this deck and its subdecks, such as `lang/french`.
        #[arg(long, short)]
        deck: Option<String>,
    },
}

/// `amgi search --help`, after the options.
const SETTINGS: &str = "\
The collection's settings are in `amgi.toml` at its root. A setting left out has the
value shown, except the new-card rules and review limit, which there are none of:

  [study]
  retention = 0.9            # how likely to remember a card when it comes due, 0.7-0.99
  day-starts-at = 4          # the hour a study day starts; before it counts as the day before

  [new]
  per-day = 20               # new cards a day in all; 0 to only review

  [[new.rule]]               # fewer new cards from those a search matches
  cards = \"deck:languages\"
  per-day = 10

  [review]
  per-day = 200              # reviews of due cards a day

  [leech]
  lapses = 8                 # times forgotten that make a card a leech
  suspend = false            # tag a leech #suspended in its file

  [lint]                     # rules for `amgi lint`
  media-links = \"absolute\"
  missing-media = true
  tags = true

A card matching several new-card rules counts toward each, so the strictest decides.
Changes apply the next time the app or `amgi` reads the collection.";

const SEARCH_SYNTAX: &str = "\
Every word must match the card's question, answer, tags or note fields, ignoring case
and accents. Words can also be filters:

  \"capital of\"              that exact phrase
  -paris                    without `paris`; works before any filter, too
  #verbs  #lang*            tagged `verbs`; with a tag starting with `lang`
  deck:french               in the deck `french` or under it, such as `lang/french/words`
  q:paris  a:\"to go\"        that text in the question or answer only
  meaning:apple             a note field of that name has the text; leave out spaces
  is:new  is:due  is:suspended  is:leech  is:cloze  is:basic  is:mature
                            in that state or of that kind; mature is 21+ days apart
  lapses>3  reviews<=5      compared by count: <, <=, >, >=, =
  due<1  due<7  due<0       due today; within 7 days; overdue. New cards have no due date
  interval>=7               next due 7+ days after its last review

Anything else, such as 12:30, is searched as text.";

fn main() -> ExitCode {
    // Stop quietly when the reader goes away, as in `amgi search | head`, like other
    // Unix tools, rather than panicking on the broken pipe.
    // SAFETY: called before any other thread starts.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
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
        Command::Lint { fix } => commands::lint(&collection, out, fix),
        Command::Fmt { check } => commands::fmt(&collection, out, check),
        Command::Search { query, ids } => commands::search(&collection, out, &query.join(" "), ids),
        Command::Decks => commands::decks(&collection, out),
        Command::Tags => commands::tags(&collection, out),
        Command::Info { card } => commands::info(&collection, out, &card.join(" ")),
        Command::Prune { trash } => commands::prune(&collection, out, trash),
        Command::Diff { revs, staged, check, reviewed } => commands::diff(&collection, out, &revs, staged, check, reviewed),
        Command::Stats { days, deck, time } => commands::stats(&collection, out, days, deck.as_deref(), time),
        Command::Forecast { new, per_day, keep_adding, days, deck } => {
            let (new, every_day) = per_day.map_or((new, false), |n| (n, true));
            commands::forecast(&collection, out, new, every_day, keep_adding, days, deck.as_deref())
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e}", style::red("amgi:"));
            ExitCode::from(2)
        }
    }
}
