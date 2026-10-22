//! Colors and emphasis for terminal output, in the terminal's own palette so they
//! suit its theme. Off when stdout isn't a terminal, with `NO_COLOR`, or with
//! `--color never`.

use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default, clap::ValueEnum)]
pub enum ColorChoice {
    /// When printing to a terminal, unless `NO_COLOR` is set.
    #[default]
    Auto,
    Always,
    Never,
}

pub fn init(choice: ColorChoice) {
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    let on = match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto if set("NO_COLOR") => false,
        ColorChoice::Auto if set("CLICOLOR_FORCE") => true,
        // SAFETY: `isatty` only reads the descriptor.
        ColorChoice::Auto => unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 },
    };
    ENABLED.store(on, Ordering::Relaxed);
}

fn paint(codes: &str, s: impl Display) -> String {
    if ENABLED.load(Ordering::Relaxed) { format!("\x1b[{codes}m{s}\x1b[0m") } else { s.to_string() }
}

/// Headings and the figures that matter.
pub fn bold(s: impl Display) -> String {
    paint("1", s)
}

/// Labels, ids and anything secondary.
pub fn dim(s: impl Display) -> String {
    paint("2", s)
}

/// Overdue cards, Again, removed cards and errors.
pub fn red(s: impl Display) -> String {
    paint("31", s)
}

/// Added cards and all clear.
pub fn green(s: impl Display) -> String {
    paint("32", s)
}

/// Due cards, Hard and changed cards.
pub fn yellow(s: impl Display) -> String {
    paint("33", s)
}

/// New cards, Easy and the accent: the app's own blue.
pub fn blue(s: impl Display) -> String {
    paint("34", s)
}

/// Paths to cards, as compilers print them.
pub fn path(s: impl Display) -> String {
    paint("36", s)
}

/// `n` right-aligned in `width` columns and drawn with `style`, or dimmed when it is
/// zero, so the counts that matter stand out. Pad before styling: escape codes take
/// no columns, so padding a styled string comes out short.
pub fn figure(n: u32, width: usize, style: impl Fn(String) -> String) -> String {
    let s = format!("{n:>width$}");
    if n == 0 { dim(s) } else { style(s) }
}
