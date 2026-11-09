//! The collection's settings, in `amgi.toml` at its root. Each one left out has its
//! default, as does everything when there is no file:
//!
//! ```toml
//! [study]
//! retention = 0.9           # how likely to remember a card when it comes due, 0.7–0.99
//! day-starts-at = 4         # the hour a study day starts, 0–23
//! parameters = [0.212, …]   # the memory model's 21, from `amgi optimize`
//!
//! [new]
//! per-day = 20              # new cards a day in all; 0 to only review
//!
//! [[new.rule]]              # fewer from the cards a search matches
//! cards = "deck:languages"
//! per-day = 10
//!
//! [review]
//! per-day = 200             # reviews of due cards a day; no limit without it
//!
//! [leech]
//! lapses = 8                # times forgotten that make a card a leech
//! suspend = false           # suspend a card when it becomes one
//!
//! [lint]                    # rules for `amgi lint`, in `lint.rs`
//! ```
//!
//! Unlike `~/.config/amgi/config.toml` (`config.rs`), these belong to the collection,
//! so they go wherever its folder goes.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::lint;
use crate::scheduler::{Parameters, Scheduler};

/// The settings file, at the collection root.
pub const FILE: &str = "amgi.toml";

/// What `amgi.toml` says.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub study: Study,
    pub new: NewCards,
    pub review: Reviews,
    pub leech: Leeches,
    pub lint: lint::Rules,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Study {
    /// Target probability of recall when a card comes due.
    pub retention: Option<f32>,
    /// The hour, local time, a new study day starts.
    pub day_starts_at: Option<u8>,
    /// The memory model's parameters, fitted to the collection's reviews; spanned so
    /// `amgi optimize` can replace them.
    pub parameters: Option<toml::Spanned<Vec<f32>>>,
}

impl Study {
    /// The parameters, once `check` has passed.
    pub fn parameters(&self) -> Option<Parameters> {
        self.parameters.as_ref().and_then(|p| p.get_ref().as_slice().try_into().ok())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct NewCards {
    /// New cards a day in all.
    pub per_day: Option<u32>,
    pub rule: Vec<NewRule>,
}

/// At most `per_day` new cards a day from those matching `cards`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct NewRule {
    /// A search; spanned to say where a rule is in the file.
    pub cards: toml::Spanned<String>,
    pub per_day: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Reviews {
    /// Reviews of due cards a day; failed cards coming back and new cards don't count.
    pub per_day: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Leeches {
    /// Lapses that make a card a leech; at least 1.
    pub lapses: Option<u32>,
    pub suspend: Option<bool>,
}

impl Settings {
    /// `amgi.toml` at `root`, or the defaults when there is none; or why it can't be read.
    pub fn load(root: &Path) -> Result<Self, String> {
        match fs::read_to_string(root.join(FILE)) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{FILE}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{FILE}: {e}")),
        }
    }

    /// The file's text as settings, or what's wrong with it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Self = toml::from_str(text).map_err(|e| e.to_string().trim_end().to_string())?;
        settings.check()?;
        Ok(settings)
    }

    fn check(&self) -> Result<(), String> {
        if let Some(r) = self.study.retention.filter(|r| !(0.7..=0.99).contains(r)) {
            return Err(format!("study.retention must be from 0.7 to 0.99, not {r}"));
        }
        if let Some(hour) = self.study.day_starts_at.filter(|&h| h > 23) {
            return Err(format!("study.day-starts-at must be an hour from 0 to 23, not {hour}"));
        }
        if let Some(p) = &self.study.parameters {
            let n = p.get_ref().len();
            let parameters = self.study.parameters().ok_or(format!("study.parameters must be 21 numbers, not {n}"))?;
            if Scheduler::new(parameters, 0.9).is_err() {
                return Err("study.parameters aren't ones the memory model can use; run `amgi optimize` to fit new ones".into());
            }
        }
        if self.leech.lapses == Some(0) {
            return Err("leech.lapses must be at least 1".into());
        }
        Ok(())
    }

    /// Whether the file gives `key` in `table`, for those `set` writes.
    pub(crate) fn has(&self, table: &str, key: &str) -> bool {
        match (table, key) {
            ("study", "retention") => self.study.retention.is_some(),
            ("study", "day-starts-at") => self.study.day_starts_at.is_some(),
            ("new", "per-day") => self.new.per_day.is_some(),
            ("review", "per-day") => self.review.per_day.is_some(),
            ("leech", "lapses") => self.leech.lapses.is_some(),
            ("leech", "suspend") => self.leech.suspend.is_some(),
            _ => false,
        }
    }
}

/// The parameters in a file with errors, so a mistake in another setting doesn't
/// put every card's memory back on the defaults: those it gives, the defaults if it
/// gives none, or `None` if they can't be read, as when the file isn't TOML.
pub(crate) fn parameters_despite_errors(text: &str) -> Option<Parameters> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct File {
        study: OnlyParameters,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct OnlyParameters {
        parameters: Option<Vec<f32>>,
    }
    let Some(parameters) = toml::from_str::<File>(text).ok()?.study.parameters else {
        return Some(fsrs::DEFAULT_PARAMETERS);
    };
    let parameters = parameters.as_slice().try_into().ok()?;
    Scheduler::new(parameters, 0.9).ok().map(|_| parameters)
}

/// `text` with `study.parameters` set to `parameters`: in place of those it has, or
/// added. `text` must be valid settings.
pub(crate) fn set_parameters(text: &str, parameters: &Parameters) -> String {
    let value = format!("[{}]", parameters.map(|p| p.to_string()).join(", "));
    match Settings::parse(text).ok().and_then(|s| s.study.parameters) {
        Some(old) => format!("{}{value}{}", &text[..old.span().start], &text[old.span().end..]),
        None => set(text, "study", "parameters", &value),
    }
}

/// `text` with `key = value` added to `[table]`: after the table's last line if the
/// file has it, else in a new one before its first `[[table.…]]` or at the end. The
/// rest of the file, comments included, stays as it was.
pub(crate) fn set(text: &str, table: &str, key: &str, value: &str) -> String {
    let setting = format!("{key} = {value}\n");
    let header = format!("[{table}]");
    let array = format!("[[{table}.");
    let is_header = |line: &str| {
        let rest = line.trim_start().strip_prefix(&header).map(str::trim_start);
        rest.is_some_and(|rest| rest.is_empty() || rest.starts_with('#'))
    };
    // Where the table's last non-blank line ends, once inside it.
    let mut table_end: Option<usize> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if table_end.is_some() && trimmed.starts_with('[') {
            break;
        }
        if is_header(line) || (table_end.is_some() && !trimmed.trim_end().is_empty()) {
            table_end = Some(at + line.len());
        } else if table_end.is_none() && trimmed.starts_with(&array) {
            return format!("{}{header}\n{setting}\n{}", &text[..at], &text[at..]);
        }
        at += line.len();
    }
    if let Some(end) = table_end {
        let newline = if text[..end].ends_with('\n') { "" } else { "\n" };
        return format!("{}{newline}{setting}{}", &text[..end], &text[end..]);
    }
    match text.trim_end() {
        "" => format!("{header}\n{setting}"),
        rest => format!("{rest}\n\n{header}\n{setting}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings() {
        let s = Settings::parse("").unwrap();
        assert_eq!((s.study.retention, s.new.per_day, s.review.per_day, s.leech.suspend), (None, None, None, None));
        let text = "[study]\nretention = 0.85\nday-starts-at = 3\n\n[new]\nper-day = 7\n\n[review]\nper-day = 100\n\n[leech]\nlapses = 4\nsuspend = true\n";
        let s = Settings::parse(text).unwrap();
        assert_eq!((s.study.retention, s.study.day_starts_at), (Some(0.85), Some(3)));
        assert_eq!((s.new.per_day, s.review.per_day), (Some(7), Some(100)));
        assert_eq!((s.leech.lapses, s.leech.suspend), (Some(4), Some(true)));
        let rules = Settings::parse("[[new.rule]]\ncards = \"deck:a\"\nper-day = 5\n").unwrap().new.rule;
        assert_eq!((rules[0].cards.get_ref().as_str(), rules[0].per_day), ("deck:a", 5));
    }

    #[test]
    fn refuses_bad_settings() {
        let err = |text: &str| Settings::parse(text).unwrap_err();
        assert!(err("[[new.rule]]\ncards = \"deck:a\"\n").contains("per-day"));
        assert!(err("[study]\nretention = 90\n").contains("from 0.7 to 0.99, not 90"));
        assert!(err("[study]\nday-starts-at = 24\n").contains("from 0 to 23"));
        assert!(err("[leech]\nlapses = 0\n").contains("at least 1"));
        assert!(err("[review]\nper-dya = 1\n").contains("unknown field"));
        assert!(err("[study]\nparameters = [1, 2]\n").contains("21 numbers, not 2"));
    }

    #[test]
    fn parameters_in_a_file_with_errors() {
        let p = fsrs::DEFAULT_PARAMETERS;
        let mut q = p;
        q[0] = 0.5;
        let given = set_parameters("", &q);
        assert_eq!(parameters_despite_errors(&format!("{given}retention = 5\n")), Some(q));
        assert_eq!(parameters_despite_errors("[leech]\nlapses = 0\n"), Some(p));
        assert_eq!(parameters_despite_errors("[study]\nparameters = [1, 2]\n"), None);
        assert_eq!(parameters_despite_errors(&format!("{given}[study\n")), None);
    }

    #[test]
    fn parameters_are_replaced_in_place() {
        let p = fsrs::DEFAULT_PARAMETERS;
        let text = set_parameters("# mine\n[study]\nretention = 0.85\n", &p);
        assert_eq!(Settings::parse(&text).unwrap().study.parameters(), Some(p));
        assert!(text.starts_with("# mine\n[study]\nretention = 0.85\nparameters = [0.212, 1.2931, "), "{text}");
        let mut q = p;
        q[0] = 0.5;
        let spread = text.replace(", ", ",\n  ") + "\n[new]\nper-day = 5\n";
        let again = set_parameters(&spread, &q);
        assert_eq!(Settings::parse(&again).unwrap().study.parameters(), Some(q));
        assert!(again.ends_with("]\n\n[new]\nper-day = 5\n") && again.matches("parameters").count() == 1, "{again}");
    }

    #[test]
    fn set_goes_in_its_table() {
        assert_eq!(set("", "new", "per-day", "5"), "[new]\nper-day = 5\n");
        assert_eq!(set("[new]\n", "new", "per-day", "5"), "[new]\nper-day = 5\n");
        assert_eq!(set("[new] # mine", "new", "per-day", "5"), "[new] # mine\nper-day = 5\n");
        assert_eq!(
            set("[lint]\ntags = false\n\n[[new.rule]]\ncards = \"a\"\nper-day = 1\n", "new", "per-day", "5"),
            "[lint]\ntags = false\n\n[new]\nper-day = 5\n\n[[new.rule]]\ncards = \"a\"\nper-day = 1\n"
        );
        assert_eq!(set("[lint]\ntags = false", "leech", "lapses", "3"), "[lint]\ntags = false\n\n[leech]\nlapses = 3\n");
        // After the table's last setting, before what follows it.
        assert_eq!(
            set("[leech]\nlapses = 3\n\n[lint]\ntags = false\n", "leech", "suspend", "true"),
            "[leech]\nlapses = 3\nsuspend = true\n\n[lint]\ntags = false\n"
        );
    }
}
