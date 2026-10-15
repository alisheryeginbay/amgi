//! amgi: plain-text flash cards scheduled with FSRS.
//!
//! A collection is a directory of Markdown files; each file is a deck. Review
//! state lives in `.amgi.sqlite` at the collection root.

pub mod fsrs;
pub mod parser;
pub mod store;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub use fsrs::Rating;
pub use parser::CardKind;
use fsrs::Scheduler;
use parser::ParsedCard;
use store::{CardState, Store};

uniffi::setup_scaffolding!();

pub const DB_FILE: &str = ".amgi.sqlite";
const DAY_SECS: i64 = 86_400;
/// The scheduling day rolls over at 4am local time, so late-night study counts as the same day.
const DAY_ROLLOVER_SECS: i64 = 4 * 3600;
/// When nothing else is left, cards due within this window are shown early.
const LEARN_AHEAD_SECS: i64 = 20 * 60;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AmgiError {
    #[error("I/O error: {message}")]
    Io { message: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("unknown card: {id}")]
    UnknownCard { id: String },
}

impl From<std::io::Error> for AmgiError {
    fn from(e: std::io::Error) -> Self {
        AmgiError::Io {
            message: e.to_string(),
        }
    }
}

impl From<rusqlite::Error> for AmgiError {
    fn from(e: rusqlite::Error) -> Self {
        AmgiError::Database {
            message: e.to_string(),
        }
    }
}

type Result<T, E = AmgiError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeckInfo {
    /// File path relative to the collection root, without the `.md` extension.
    pub name: String,
    pub total: u32,
    pub due: u32,
    /// New cards available today, capped by the daily new-card limit.
    pub new: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StudyCard {
    pub id: String,
    pub deck: String,
    /// Source file relative to the collection root.
    pub path: String,
    pub line: u32,
    pub kind: CardKind,
    /// Markdown.
    pub front: String,
    /// Markdown.
    pub back: String,
    pub is_new: bool,
    /// Seconds until the card is due again for Again, Hard, Good, Easy.
    pub intervals: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CollectionWarning {
    pub path: String,
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodayStats {
    pub reviewed: u32,
    pub new_studied: u32,
}

struct Card {
    parsed: ParsedCard,
    deck: String,
    path: String,
}

struct Inner {
    root: PathBuf,
    store: Store,
    scheduler: Scheduler,
    cards: Vec<Card>,
    index: HashMap<String, usize>,
    states: HashMap<String, CardState>,
    warnings: Vec<CollectionWarning>,
    new_per_day: u32,
    utc_offset_secs: i64,
}

#[derive(uniffi::Object)]
pub struct CardCollection {
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl CardCollection {
    /// Open the collection rooted at `root`, creating its database if needed.
    #[uniffi::constructor]
    pub fn open(root: String) -> Result<Arc<Self>> {
        let root = PathBuf::from(root);
        let store = Store::open(&root.join(DB_FILE))?;
        Self::with_store(root, store).map(Arc::new)
    }

    /// Re-read all Markdown files. Call after files change on disk.
    pub fn reload(&self) -> Result<()> {
        self.lock().reload()
    }

    pub fn set_new_cards_per_day(&self, limit: u32) {
        self.lock().new_per_day = limit;
    }

    /// Local time zone offset, used to decide when the study day rolls over.
    pub fn set_utc_offset(&self, seconds: i32) {
        self.lock().utc_offset_secs = seconds.into();
    }

    /// Target probability of recall when a card comes due, between 0.7 and 0.99.
    pub fn set_desired_retention(&self, retention: f64) {
        self.lock().scheduler.desired_retention = retention.clamp(0.7, 0.99);
    }

    pub fn decks(&self) -> Result<Vec<DeckInfo>> {
        self.lock().decks(unix_now())
    }

    /// The next card to study, from one deck or (with `None`) the whole collection.
    pub fn next_card(&self, deck: Option<String>) -> Result<Option<StudyCard>> {
        self.lock().next_card(deck.as_deref(), unix_now())
    }

    pub fn answer(&self, card_id: String, rating: Rating) -> Result<()> {
        self.lock().answer(&card_id, rating, unix_now())
    }

    pub fn today(&self) -> Result<TodayStats> {
        self.lock().today(unix_now())
    }

    pub fn warnings(&self) -> Vec<CollectionWarning> {
        self.lock().warnings.clone()
    }
}

impl CardCollection {
    fn with_store(root: PathBuf, store: Store) -> Result<Self> {
        let states = store.states()?;
        let mut inner = Inner {
            root,
            store,
            scheduler: Scheduler::default(),
            cards: Vec::new(),
            index: HashMap::new(),
            states,
            warnings: Vec::new(),
            new_per_day: 20,
            utc_offset_secs: 0,
        };
        inner.reload()?;
        Ok(CardCollection {
            inner: Mutex::new(inner),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic mid-review leaves no partial state worth discarding: SQLite writes are transactional.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inner {
    fn reload(&mut self) -> Result<()> {
        let mut files = Vec::new();
        collect_markdown(&self.root, &mut files)?;
        files.sort();

        self.cards.clear();
        self.index.clear();
        self.warnings.clear();
        for file in files {
            let rel = relative(&self.root, &file);
            let deck = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            let result = parser::parse(&fs::read_to_string(&file)?);
            for w in result.warnings {
                self.warnings.push(CollectionWarning {
                    path: rel.clone(),
                    line: w.line,
                    message: w.message,
                });
            }
            for parsed in result.cards {
                if let Some(&i) = self.index.get(&parsed.id) {
                    let first = &self.cards[i];
                    self.warnings.push(CollectionWarning {
                        path: rel.clone(),
                        line: parsed.line,
                        message: format!("duplicate of {}:{}", first.path, first.parsed.line),
                    });
                    continue;
                }
                self.index.insert(parsed.id.clone(), self.cards.len());
                self.cards.push(Card {
                    parsed,
                    deck: deck.clone(),
                    path: rel.clone(),
                });
            }
        }
        Ok(())
    }

    fn day_start(&self, now: i64) -> i64 {
        let local = now + self.utc_offset_secs - DAY_ROLLOVER_SECS;
        local.div_euclid(DAY_SECS) * DAY_SECS + DAY_ROLLOVER_SECS - self.utc_offset_secs
    }

    fn new_remaining(&self, now: i64) -> Result<u32> {
        let studied = self.store.new_cards_since(self.day_start(now))?;
        Ok(self.new_per_day.saturating_sub(studied))
    }

    fn in_deck<'a>(&'a self, deck: Option<&'a str>) -> impl Iterator<Item = &'a Card> + 'a {
        self.cards
            .iter()
            .filter(move |c| deck.is_none_or(|d| c.deck == d))
    }

    fn decks(&self, now: i64) -> Result<Vec<DeckInfo>> {
        let remaining = self.new_remaining(now)?;
        let mut decks: Vec<DeckInfo> = Vec::new();
        for card in &self.cards {
            if decks.last().is_none_or(|d| d.name != card.deck) {
                decks.push(DeckInfo {
                    name: card.deck.clone(),
                    total: 0,
                    due: 0,
                    new: 0,
                });
            }
            let deck = decks.last_mut().expect("pushed above");
            deck.total += 1;
            match self.states.get(&card.parsed.id) {
                Some(s) if s.due <= now => deck.due += 1,
                Some(_) => {}
                None => deck.new += 1,
            }
        }
        for deck in &mut decks {
            deck.new = deck.new.min(remaining);
        }
        Ok(decks)
    }

    fn next_card(&self, deck: Option<&str>, now: i64) -> Result<Option<StudyCard>> {
        let earliest_due_before = |limit: i64| {
            self.in_deck(deck)
                .filter_map(|c| self.states.get(&c.parsed.id).map(|s| (s.due, c)))
                .filter(|(due, _)| *due <= limit)
                .min_by_key(|(due, _)| *due)
                .map(|(_, c)| c)
        };

        let card = match earliest_due_before(now) {
            Some(c) => Some(c),
            None if self.new_remaining(now)? > 0 => self
                .in_deck(deck)
                .find(|c| !self.states.contains_key(&c.parsed.id))
                .or_else(|| earliest_due_before(now + LEARN_AHEAD_SECS)),
            None => earliest_due_before(now + LEARN_AHEAD_SECS),
        };
        Ok(card.map(|c| self.study_card(c, now)))
    }

    fn elapsed(&self, state: Option<&CardState>, now: i64) -> (Option<fsrs::Memory>, f64) {
        match state {
            Some(s) => (
                Some(s.memory),
                (now - s.last_review).max(0) as f64 / DAY_SECS as f64,
            ),
            None => (None, 0.0),
        }
    }

    fn study_card(&self, card: &Card, now: i64) -> StudyCard {
        let state = self.states.get(&card.parsed.id);
        let (memory, elapsed) = self.elapsed(state, now);
        let preview = self.scheduler.preview(memory, elapsed);
        StudyCard {
            id: card.parsed.id.clone(),
            deck: card.deck.clone(),
            path: card.path.clone(),
            line: card.parsed.line,
            kind: card.parsed.kind,
            front: card.parsed.front.clone(),
            back: card.parsed.back.clone(),
            is_new: state.is_none(),
            intervals: preview.iter().map(|o| o.interval_secs).collect(),
        }
    }

    fn answer(&mut self, card_id: &str, rating: Rating, now: i64) -> Result<()> {
        if !self.index.contains_key(card_id) {
            return Err(AmgiError::UnknownCard { id: card_id.into() });
        }
        let prev = self.states.get(card_id).copied();
        let (memory, elapsed) = self.elapsed(prev.as_ref(), now);
        let outcome = self.scheduler.schedule(memory, elapsed, rating);
        let state = self
            .store
            .record_review(card_id, now, rating, elapsed, prev, outcome)?;
        self.states.insert(card_id.to_string(), state);
        Ok(())
    }

    fn today(&self, now: i64) -> Result<TodayStats> {
        let since = self.day_start(now);
        Ok(TodayStats {
            reviewed: self.store.reviews_since(since)?,
            new_studied: self.store.new_cards_since(since)?,
        })
    }
}

/// Short human-readable interval for answer buttons: `10m`, `3d`, `1.5mo`, `2.1y`.
#[uniffi::export]
pub fn format_interval(seconds: i64) -> String {
    let s = seconds.max(0) as f64;
    let (value, unit) = match s {
        s if s < 3600.0 => ((s / 60.0).max(1.0), "m"),
        s if s < 86_400.0 => (s / 3600.0, "h"),
        s if s < 30.0 * 86_400.0 => (s / 86_400.0, "d"),
        s if s < 365.0 * 86_400.0 => (s / (30.0 * 86_400.0), "mo"),
        s => (s / (365.0 * 86_400.0), "y"),
    };
    if value < 10.0 && unit != "m" && unit != "d" && value.fract() >= 0.05 {
        format!("{value:.1}{unit}")
    } else {
        format!("{}{unit}", value.round() as i64)
    }
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_markdown(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_760_000_000;

    fn collection(files: &[(&str, &str)]) -> (PathBuf, CardCollection) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("amgi-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (name, body) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        let c = CardCollection::with_store(dir.clone(), Store::open_in_memory().unwrap()).unwrap();
        (dir, c)
    }

    #[test]
    fn study_flow() {
        let (dir, c) = collection(&[
            ("a.md", "Q: 1\nA: one\n\nQ: 2\nA: two\n"),
            ("sub/b.md", "C: [x] and [y]\n\nQ: 1\nA: dup\n"),
            (".hidden/c.md", "Q: hidden\nA: no\n"),
        ]);
        let mut inner = c.lock();
        assert_eq!(inner.cards.len(), 4);
        assert_eq!(inner.warnings.len(), 1);
        assert!(inner.warnings[0].message.contains("duplicate of a.md:1"));

        let decks = inner.decks(T0).unwrap();
        let names: Vec<_> = decks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["a", "sub/b"]);
        assert_eq!((decks[0].total, decks[0].new, decks[0].due), (2, 2, 0));

        // New cards come in file order.
        let card = inner.next_card(Some("a"), T0).unwrap().unwrap();
        assert_eq!(card.front, "1");
        assert!(card.is_new);
        assert_eq!(card.intervals.len(), 4);
        inner.answer(&card.id, Rating::Good, T0).unwrap();

        let card2 = inner.next_card(Some("a"), T0).unwrap().unwrap();
        assert_eq!(card2.front, "2");
        inner.answer(&card2.id, Rating::Again, T0).unwrap();

        // The failed card is shown again within the learn-ahead window.
        let again = inner.next_card(Some("a"), T0 + 60).unwrap().unwrap();
        assert_eq!(again.id, card2.id);
        assert!(!again.is_new);

        // Two days later the first card is due.
        let decks = inner.decks(T0 + 2 * DAY_SECS).unwrap();
        assert_eq!((decks[0].due, decks[0].new), (2, 0));
        assert_eq!(inner.today(T0).unwrap().new_studied, 2);
        assert!(matches!(
            inner.answer("nope", Rating::Good, T0),
            Err(AmgiError::UnknownCard { .. })
        ));
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_card_limit() {
        let (dir, c) = collection(&[("a.md", "Q: 1\nA: 1\n\nQ: 2\nA: 2\n\nQ: 3\nA: 3\n")]);
        c.set_new_cards_per_day(1);
        let mut inner = c.lock();
        let card = inner.next_card(None, T0).unwrap().unwrap();
        inner.answer(&card.id, Rating::Easy, T0).unwrap();
        assert!(inner.next_card(None, T0).unwrap().is_none());
        assert_eq!(inner.decks(T0).unwrap()[0].new, 0);
        // Tomorrow the limit resets.
        assert!(inner.next_card(None, T0 + DAY_SECS).unwrap().is_some());
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn day_rollover_respects_offset() {
        let (dir, c) = collection(&[("a.md", "")]);
        c.set_utc_offset(5 * 3600);
        let inner = c.lock();
        // 2025-10-09 03:00 local (+05:00) is still the previous study day.
        let start = inner.day_start(1_759_960_800);
        assert_eq!((start + 5 * 3600 - DAY_ROLLOVER_SECS).rem_euclid(DAY_SECS), 0);
        assert!(start <= 1_759_960_800 && 1_759_960_800 - start < DAY_SECS);
        drop(inner);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn intervals_format() {
        assert_eq!(format_interval(600), "10m");
        assert_eq!(format_interval(5400), "1.5h");
        assert_eq!(format_interval(86_400 * 3), "3d");
        assert_eq!(format_interval(86_400 * 45), "1.5mo");
        assert_eq!(format_interval(86_400 * 800), "2.2y");
    }
}
