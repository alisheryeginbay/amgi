//! SQLite persistence. Card content lives in Markdown; the database only holds
//! scheduling state and the review log, keyed by content-derived card ids.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};

use crate::SavedView;
use crate::scheduler::{Memory, Outcome, Rating, ReviewState};

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE card_state (
    card_id     TEXT PRIMARY KEY,
    stability   REAL NOT NULL,
    difficulty  REAL NOT NULL,
    due         INTEGER NOT NULL,
    last_review INTEGER NOT NULL,
    reps        INTEGER NOT NULL,
    lapses      INTEGER NOT NULL
);
CREATE TABLE review_log (
    id            INTEGER PRIMARY KEY,
    card_id       TEXT NOT NULL,
    reviewed_at   INTEGER NOT NULL,
    rating        INTEGER NOT NULL,
    elapsed_days  REAL NOT NULL,
    stability     REAL NOT NULL,
    difficulty    REAL NOT NULL,
    interval_secs INTEGER NOT NULL,
    was_new       INTEGER NOT NULL
);
CREATE INDEX review_log_card ON review_log (card_id, reviewed_at);
CREATE INDEX review_log_time ON review_log (reviewed_at);
"#, r#"
CREATE TABLE tag_icon (
    tag  TEXT PRIMARY KEY,
    icon TEXT NOT NULL
);
"#, r#"
CREATE TABLE saved_view (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL,
    query TEXT NOT NULL,
    icon  TEXT
);
INSERT INTO saved_view (name, query, icon) VALUES ('Leeches', 'is:leech', '🐞');
"#, r#"
-- `ReviewState`: 0 New, 1 Learning, 2 Review, 3 Relearning; NULL for reviews logged
-- before this column. A row with rating 0 is a reset: the card became new again.
ALTER TABLE review_log ADD COLUMN review_state INTEGER;
"#, r#"
-- With the rating, counting reviews per day reads only the index.
DROP INDEX review_log_time;
CREATE INDEX review_log_time ON review_log (reviewed_at, rating);
"#, r#"
-- Cards imported from Anki were due at the start of Anki's day, often 2 am, which
-- falls in the previous amgi study day. Make them due whole days after their last
-- review, as amgi schedules: the same calendar day, at the time of that review.
-- Anki's days start on the quarter hour; amgi's own dues are whole days or ten
-- minutes after the review.
UPDATE card_state
SET due = last_review + (due - last_review + 86399) / 86400 * 86400
WHERE due - last_review > 900 AND (due - last_review) % 86400 != 0 AND due % 900 = 0;
"#, r#"
-- Scheduling settings that belong to the collection, such as `desired_retention`.
CREATE TABLE setting (
    key   TEXT PRIMARY KEY,
    value NOT NULL
);
"#];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardState {
    pub memory: Memory,
    /// Unix seconds.
    pub due: i64,
    pub last_review: i64,
    pub reps: u32,
    pub lapses: u32,
    /// The interval the latest review scheduled. Under a day while a failed card
    /// waits to come back.
    pub interval_secs: i64,
}

/// The interval of a card's latest review, for a `card_state` query.
const LAST_INTERVAL: &str = "COALESCE((SELECT interval_secs FROM review_log l
    WHERE l.card_id = card_state.card_id AND rating > 0
    ORDER BY reviewed_at DESC, id DESC LIMIT 1), due - last_review)";

/// One row of the review log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggedReview {
    pub reviewed_at: i64,
    pub rating: Rating,
    pub elapsed_days: f64,
    /// Memory state after the review.
    pub memory: Memory,
    pub interval_secs: i64,
    pub was_new: bool,
    /// `None` for reviews logged before amgi recorded it.
    pub state: Option<ReviewState>,
}

/// An entry to write to the review log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogEntry {
    Review(LoggedReview),
    /// The card was reset to new at this time (Unix seconds).
    Reset(i64),
}

pub struct Store {
    conn: Connection,
    /// The database file and its identity when opened; `None` in memory.
    file: Option<(PathBuf, FileId)>,
}

/// Device and inode: which file a path named when it was opened.
type FileId = (u64, u64);

fn file_id(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Keeps the reviews of the cards in `:cards`, or every review when `:all` is set. The
/// array is bound as a pointer, which SQL reads as `NULL`, hence the separate flag.
const IN_CARDS: &str = "(:all OR card_id IN rarray(:cards))";

/// `:all` and `:cards` for `IN_CARDS`; `None` keeps every card.
fn card_list(cards: Option<&[String]>) -> (bool, Rc<Vec<Value>>) {
    let ids = cards.unwrap_or_default().iter().cloned().map(Value::from).collect();
    (cards.is_none(), Rc::new(ids))
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let mut store = Self::init(Connection::open(path)?)?;
        store.file = file_id(path).map(|id| (path.to_path_buf(), id));
        Ok(store)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// The database's path, if the file there is no longer the one this store has
    /// open: it was deleted, or replaced by a sync, restore or checkout. Writes would
    /// go to the old file and be lost.
    pub fn replaced(&self) -> Option<&Path> {
        let (path, id) = self.file.as_ref()?;
        (file_id(path) != Some(*id)).then_some(path.as_path())
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        // `rarray(?)`, for queries limited to some cards.
        rusqlite::vtab::array::load_module(&conn)?;
        // Rollback journal instead of WAL: the database stays a single file, which is
        // safer in synced folders, and one small write per review needs no concurrency.
        // Also converts databases created in WAL mode by earlier versions.
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Store { conn, file: None })
    }

    pub fn states(&self) -> rusqlite::Result<HashMap<String, CardState>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT card_id, stability, difficulty, due, last_review, reps, lapses, {LAST_INTERVAL} FROM card_state"
        ))?;
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                CardState {
                    memory: Memory {
                        stability: r.get(1)?,
                        difficulty: r.get(2)?,
                    },
                    due: r.get(3)?,
                    last_review: r.get(4)?,
                    reps: r.get(5)?,
                    lapses: r.get(6)?,
                    interval_secs: r.get(7)?,
                },
            ))
        })?
        .collect()
    }

    pub fn state(&self, card_id: &str) -> rusqlite::Result<Option<CardState>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT stability, difficulty, due, last_review, reps, lapses, {LAST_INTERVAL}
                     FROM card_state WHERE card_id = ?1"
                ),
                [card_id],
                |r| {
                    Ok(CardState {
                        memory: Memory {
                            stability: r.get(0)?,
                            difficulty: r.get(1)?,
                        },
                        due: r.get(2)?,
                        last_review: r.get(3)?,
                        reps: r.get(4)?,
                        lapses: r.get(5)?,
                        interval_secs: r.get(6)?,
                    })
                },
            )
            .optional()
    }

    /// A card's review log, oldest first. Rows with an unknown rating are skipped.
    pub fn reviews(&self, card_id: &str) -> rusqlite::Result<Vec<LoggedReview>> {
        let mut stmt = self.conn.prepare(
            "SELECT reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new, review_state
             FROM review_log WHERE card_id = ?1 ORDER BY reviewed_at, id",
        )?;
        let rows = stmt.query_map([card_id], |r| {
            let Some(rating) = Rating::from_number(r.get(1)?) else { return Ok(None) };
            Ok(Some(LoggedReview {
                reviewed_at: r.get(0)?,
                rating,
                elapsed_days: r.get(2)?,
                memory: Memory {
                    stability: r.get(3)?,
                    difficulty: r.get(4)?,
                },
                interval_secs: r.get(5)?,
                was_new: r.get(6)?,
                state: r.get::<_, Option<i64>>(7)?.and_then(ReviewState::from_number),
            }))
        })?;
        rows.filter_map(Result::transpose).collect()
    }

    /// Stage of the card's latest review; `None` if it has none or it predates the
    /// `review_state` column.
    pub fn last_review_state(&self, card_id: &str) -> rusqlite::Result<Option<ReviewState>> {
        let state: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT review_state FROM review_log WHERE card_id = ?1 AND rating > 0
                 ORDER BY reviewed_at DESC, id DESC LIMIT 1",
                [card_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(state.flatten().and_then(ReviewState::from_number))
    }

    /// Persist a review: append to the log and upsert the card's state atomically.
    /// Returns the new state and the review's log row, for `undo_review`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_review(
        &mut self,
        card_id: &str,
        now: i64,
        rating: Rating,
        elapsed_days: f64,
        review_state: ReviewState,
        prev: Option<CardState>,
        outcome: Outcome,
    ) -> rusqlite::Result<(CardState, i64)> {
        let lapse = u32::from(prev.is_some() && rating == Rating::Again);
        let state = CardState {
            memory: outcome.memory,
            due: now + outcome.interval_secs,
            last_review: now,
            reps: prev.map_or(0, |p| p.reps) + 1,
            lapses: prev.map_or(0, |p| p.lapses) + lapse,
            interval_secs: outcome.interval_secs,
        };
        let tx = self.conn.transaction()?;
        insert_review(
            &tx,
            card_id,
            &LoggedReview {
                reviewed_at: now,
                rating,
                elapsed_days,
                memory: state.memory,
                interval_secs: outcome.interval_secs,
                was_new: prev.is_none(),
                state: Some(review_state),
            },
        )?;
        let review_id = tx.last_insert_rowid();
        upsert_state(&tx, card_id, &state)?;
        tx.commit()?;
        Ok((state, review_id))
    }

    /// Take back a review `record_review` logged: remove its log row and put the card's
    /// state back to `prev`, or make it new again if it had none.
    pub fn undo_review(&mut self, card_id: &str, review_id: i64, prev: Option<CardState>) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM review_log WHERE id = ?1 AND card_id = ?2", params![review_id, card_id])?;
        match prev {
            Some(state) => upsert_state(&tx, card_id, &state)?,
            None => {
                tx.execute("DELETE FROM card_state WHERE card_id = ?1", [card_id])?;
            }
        }
        tx.commit()
    }

    /// Persist the history and resulting state of many cards in one transaction.
    pub fn import(&mut self, cards: &[(String, Vec<LogEntry>, CardState)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for (card_id, entries, state) in cards {
            for entry in entries {
                match entry {
                    LogEntry::Review(review) => insert_review(&tx, card_id, review)?,
                    LogEntry::Reset(at) => insert_reset(&tx, card_id, *at)?,
                }
            }
            upsert_state(&tx, card_id, state)?;
        }
        tx.commit()
    }

    /// How many reviews each of `ids` has, resets left out; ids without any are left out.
    pub fn review_counts(&self, ids: &[String]) -> rusqlite::Result<HashMap<String, u32>> {
        let ids = Rc::new(ids.iter().cloned().map(Value::from).collect::<Vec<_>>());
        self.conn
            .prepare("SELECT card_id, COUNT(*) FROM review_log WHERE rating > 0 AND card_id IN rarray(?1) GROUP BY card_id")?
            .query_map([ids], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    /// Move each card's history from its first id to its second, for cards whose id
    /// changed. A card with history under both keeps all its reviews and the state of
    /// the latest. Returns how many cards had history to move.
    pub fn move_history(&mut self, moves: &[(String, String)]) -> rusqlite::Result<usize> {
        let from = Rc::new(moves.iter().map(|(old, _)| Value::from(old.clone())).collect::<Vec<_>>());
        let with_history: std::collections::HashSet<String> = self
            .conn
            .prepare(
                "SELECT card_id FROM card_state WHERE card_id IN rarray(?1)
                 UNION SELECT card_id FROM review_log WHERE card_id IN rarray(?1)",
            )?
            .query_map([from], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if with_history.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let last_review = |id: &str| -> rusqlite::Result<Option<i64>> {
            tx.query_row("SELECT last_review FROM card_state WHERE card_id = ?1", [id], |r| r.get(0)).optional()
        };
        for (old, new) in moves.iter().filter(|(old, _)| with_history.contains(old)) {
            tx.execute("UPDATE review_log SET card_id = ?2 WHERE card_id = ?1", params![old, new])?;
            if let Some(old_last) = last_review(old)? {
                if last_review(new)?.is_some_and(|new_last| new_last >= old_last) {
                    tx.execute("DELETE FROM card_state WHERE card_id = ?1", [old])?;
                } else {
                    tx.execute("DELETE FROM card_state WHERE card_id = ?1", [new])?;
                    tx.execute("UPDATE card_state SET card_id = ?2 WHERE card_id = ?1", params![old, new])?;
                }
            }
        }
        tx.commit()?;
        Ok(with_history.len())
    }

    /// Number of new cards first reviewed at or after `since`.
    pub fn new_cards_since(&self, since: i64, cards: Option<&[String]>) -> rusqlite::Result<u32> {
        let (all, ids) = card_list(cards);
        self.conn.query_row(
            &format!("SELECT COUNT(*) FROM review_log WHERE was_new AND reviewed_at >= :since AND {IN_CARDS}"),
            named_params! { ":since": since, ":all": all, ":cards": ids },
            |r| r.get(0),
        )
    }

    /// Review counts per day at or after `since`, where day `n` starts at
    /// `n * 86400 - shift` seconds. Days without reviews are absent.
    pub fn reviews_per_day(&self, since: i64, shift: i64, cards: Option<&[String]>) -> rusqlite::Result<Vec<(i64, u32)>> {
        let (all, ids) = card_list(cards);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT (reviewed_at + :shift) / 86400 AS day, COUNT(*) FROM review_log
             WHERE rating > 0 AND reviewed_at >= :since AND {IN_CARDS} GROUP BY day ORDER BY day"
        ))?;
        stmt.query_map(named_params! { ":since": since, ":shift": shift, ":all": all, ":cards": ids }, |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    /// Reviews of cards the review before scheduled a day or more ahead, at or after
    /// `since`, as Anki counts them: per study day (as in `reviews_per_day`) and by
    /// whether the card was mature, scheduled `mature_secs` or more ahead.
    /// `(day, mature, passed, failed)`, where failed is rated Again.
    pub fn recall_per_day(
        &self,
        since: i64,
        shift: i64,
        mature_secs: i64,
        cards: Option<&[String]>,
    ) -> rusqlite::Result<Vec<(i64, bool, u32, u32)>> {
        let (all, ids) = card_list(cards);
        // Only the cards reviewed since, but all their reviews, for the interval before.
        let mut stmt = self.conn.prepare(&format!(
            "SELECT (reviewed_at + :shift) / 86400 AS day, last_interval >= :mature AS mature,
                    SUM(rating > 1), SUM(rating = 1)
             FROM (SELECT reviewed_at, rating, was_new,
                          COALESCE(LAG(interval_secs) OVER (PARTITION BY card_id ORDER BY reviewed_at, id),
                                   CAST(elapsed_days * 86400 AS INTEGER)) AS last_interval
                   FROM review_log
                   WHERE rating > 0 AND {IN_CARDS} AND card_id IN
                         (SELECT card_id FROM review_log WHERE reviewed_at >= :since AND rating > 0))
             WHERE NOT was_new AND last_interval >= 86400 AND reviewed_at >= :since
             GROUP BY day, mature ORDER BY day"
        ))?;
        stmt.query_map(
            named_params! { ":since": since, ":shift": shift, ":mature": mature_secs, ":all": all, ":cards": ids },
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?
        .collect()
    }

    /// Reviews of cards that came due after a day or more, at or after `since`: the
    /// ones the daily review limit counts. Not new cards, nor failed ones coming back.
    pub fn due_reviews_since(&self, since: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM review_log WHERE rating > 0 AND reviewed_at >= ?1 AND review_state = ?2",
            params![since, ReviewState::Review as i64],
            |r| r.get(0),
        )
    }

    pub fn reviews_since(&self, since: i64, cards: Option<&[String]>) -> rusqlite::Result<u32> {
        let (all, ids) = card_list(cards);
        self.conn.query_row(
            &format!("SELECT COUNT(*) FROM review_log WHERE rating > 0 AND reviewed_at >= :since AND {IN_CARDS}"),
            named_params! { ":since": since, ":all": all, ":cards": ids },
            |r| r.get(0),
        )
    }

    /// A setting's value, if the collection has one.
    pub fn setting(&self, key: &str) -> rusqlite::Result<Option<f64>> {
        self.conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: f64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn remove_setting(&self, key: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM setting WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Lowercased tag → icon.
    pub fn tag_icons(&self) -> rusqlite::Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT tag, icon FROM tag_icon")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    /// `None` removes the icon. `tag` is stored lowercased.
    pub fn set_tag_icon(&self, tag: &str, icon: Option<&str>) -> rusqlite::Result<()> {
        let tag = tag.to_lowercase();
        match icon {
            Some(icon) => self.conn.execute(
                "INSERT INTO tag_icon (tag, icon) VALUES (?1, ?2)
                 ON CONFLICT (tag) DO UPDATE SET icon = excluded.icon",
                params![tag, icon],
            )?,
            None => self.conn.execute("DELETE FROM tag_icon WHERE tag = ?1", [tag])?,
        };
        Ok(())
    }

    pub fn saved_views(&self) -> rusqlite::Result<Vec<SavedView>> {
        let mut stmt = self.conn.prepare("SELECT id, name, query, icon FROM saved_view ORDER BY id")?;
        stmt.query_map([], |r| Ok(SavedView { id: r.get(0)?, name: r.get(1)?, query: r.get(2)?, icon: r.get(3)? }))?
            .collect()
    }

    pub fn add_saved_view(&self, name: &str, query: &str) -> rusqlite::Result<i64> {
        self.conn.execute("INSERT INTO saved_view (name, query) VALUES (?1, ?2)", [name, query])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_saved_view(&self, view: &SavedView) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE saved_view SET name = ?2, query = ?3, icon = ?4 WHERE id = ?1",
            params![view.id, view.name, view.query, view.icon],
        )?;
        Ok(())
    }

    pub fn delete_saved_view(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM saved_view WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn insert_review(tx: &Transaction, card_id: &str, r: &LoggedReview) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO review_log
           (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new, review_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            card_id,
            r.reviewed_at,
            r.rating as i64,
            r.elapsed_days,
            r.memory.stability,
            r.memory.difficulty,
            r.interval_secs,
            r.was_new,
            r.state.map(|s| s as i64),
        ],
    )?;
    Ok(())
}

fn insert_reset(tx: &Transaction, card_id: &str, at: i64) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO review_log
           (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new)
         VALUES (?1, ?2, 0, 0, 0, 0, 0, 0)",
        params![card_id, at],
    )?;
    Ok(())
}

fn upsert_state(tx: &Transaction, card_id: &str, state: &CardState) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO card_state (card_id, stability, difficulty, due, last_review, reps, lapses)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (card_id) DO UPDATE SET
           stability = excluded.stability, difficulty = excluded.difficulty,
           due = excluded.due, last_review = excluded.last_review,
           reps = excluded.reps, lapses = excluded.lapses",
        params![
            card_id,
            state.memory.stability,
            state.memory.difficulty,
            state.due,
            state.last_review,
            state.reps,
            state.lapses,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_views() {
        let store = Store::open_in_memory().unwrap();
        let leeches = SavedView { id: 1, name: "Leeches".into(), query: "is:leech".into(), icon: Some("🐞".into()) };
        assert_eq!(store.saved_views().unwrap(), [leeches.clone()]);

        let id = store.add_saved_view("Due soon", "due<3").unwrap();
        let mut due = SavedView { id, name: "Due soon".into(), query: "due<3".into(), icon: None };
        assert_eq!(store.saved_views().unwrap(), [leeches.clone(), due.clone()]);

        due.query = "due<7 -is:suspended".into();
        due.icon = Some("⏰".into());
        store.update_saved_view(&due).unwrap();
        store.delete_saved_view(leeches.id).unwrap();
        assert_eq!(store.saved_views().unwrap(), [due]);
    }

    #[test]
    fn tag_icons() {
        let store = Store::open_in_memory().unwrap();
        store.set_tag_icon("Korean", Some("🇰🇷")).unwrap();
        store.set_tag_icon("korean", Some("🐯")).unwrap();
        store.set_tag_icon("verbs", Some("🏃")).unwrap();
        store.set_tag_icon("VERBS", None).unwrap();
        assert_eq!(store.tag_icons().unwrap(), HashMap::from([("korean".into(), "🐯".into())]));
    }

    #[test]
    fn record_and_reload() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = Outcome {
            memory: Memory {
                stability: 2.0,
                difficulty: 5.0,
            },
            interval_secs: 86_400,
        };
        let (s1, _) = store
            .record_review("abc", 1000, Rating::Good, 0.0, ReviewState::New, None, outcome)
            .unwrap();
        assert_eq!((s1.reps, s1.lapses, s1.due), (1, 0, 87_400));

        let (s2, _) = store
            .record_review("abc", 90_000, Rating::Again, 1.0, ReviewState::Review, Some(s1), outcome)
            .unwrap();
        assert_eq!((s2.reps, s2.lapses), (2, 1));
        assert_eq!(store.state("abc").unwrap(), Some(s2));
        assert_eq!(store.states().unwrap().len(), 1);
        assert_eq!(store.new_cards_since(0, None).unwrap(), 1);
        assert_eq!(store.reviews_since(0, None).unwrap(), 2);
        assert_eq!(store.reviews_since(5000, None).unwrap(), 1);
        let log = store.reviews("abc").unwrap();
        assert_eq!(log.iter().map(|r| r.rating).collect::<Vec<_>>(), [Rating::Good, Rating::Again]);
        assert!(log[0].was_new && !log[1].was_new);
        assert_eq!(log.iter().map(|r| r.state).collect::<Vec<_>>(), [Some(ReviewState::New), Some(ReviewState::Review)]);
        assert_eq!(store.last_review_state("abc").unwrap(), Some(ReviewState::Review));
        assert_eq!(store.last_review_state("other").unwrap(), None);
        assert!(store.reviews("other").unwrap().is_empty());
    }

    #[test]
    fn undo_reviews() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = |interval_secs| Outcome { memory: Memory { stability: 2.0, difficulty: 5.0 }, interval_secs };
        let (s1, r1) = store
            .record_review("abc", 1000, Rating::Good, 0.0, ReviewState::New, None, outcome(86_400))
            .unwrap();
        let (_, r2) = store
            .record_review("abc", 90_000, Rating::Again, 1.0, ReviewState::Review, Some(s1), outcome(600))
            .unwrap();

        store.undo_review("abc", r2, Some(s1)).unwrap();
        assert_eq!(store.state("abc").unwrap(), Some(s1));
        assert_eq!(store.reviews_since(0, None).unwrap(), 1);
        assert_eq!(store.last_review_state("abc").unwrap(), Some(ReviewState::New));

        store.undo_review("abc", r1, None).unwrap();
        assert_eq!(store.state("abc").unwrap(), None);
        assert!(store.reviews("abc").unwrap().is_empty());
    }

    #[test]
    fn resets_are_logged_but_not_counted() {
        let mut store = Store::open_in_memory().unwrap();
        let review = LoggedReview {
            reviewed_at: 1000,
            rating: Rating::Good,
            elapsed_days: 0.0,
            memory: Memory { stability: 2.0, difficulty: 5.0 },
            interval_secs: 86_400,
            was_new: true,
            state: Some(ReviewState::New),
        };
        let relearned = LoggedReview { reviewed_at: 3000, ..review };
        let state = CardState { memory: review.memory, due: 89_400, last_review: 3000, reps: 1, lapses: 0, interval_secs: 86_400 };
        let entries = vec![LogEntry::Review(review), LogEntry::Reset(2000), LogEntry::Review(relearned)];
        store.import(&[("abc".into(), entries, state)]).unwrap();

        let rows: i64 = store.conn.query_row("SELECT COUNT(*) FROM review_log", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 3);
        assert_eq!(store.reviews("abc").unwrap(), [review, relearned]);
        assert_eq!(store.reviews_since(0, None).unwrap(), 2);
        assert_eq!(store.new_cards_since(0, None).unwrap(), 2);
        assert_eq!(store.reviews_per_day(0, 0, None).unwrap(), [(0, 2)]);
        assert_eq!(store.last_review_state("abc").unwrap(), Some(ReviewState::New));
    }

    #[test]
    fn older_reviews_have_no_state() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute(
            "INSERT INTO review_log
               (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new)
             VALUES ('abc', 1000, 3, 0, 2, 5, 86400, 1)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        let store = Store::init(conn).unwrap();
        assert_eq!(store.reviews("abc").unwrap()[0].state, None);
        assert_eq!(store.last_review_state("abc").unwrap(), None);
    }

    #[test]
    fn anki_due_dates_move_to_whole_days() {
        let conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..5] {
            conn.execute_batch(sql).unwrap();
        }
        // Reviewed 2026-10-12 at 16:55 (UTC+5): imported due at the start of Anki's day on
        // the 22nd at 2 am, scheduled by amgi ten days on, and failed ten minutes ago.
        let last = 1_791_806_155;
        for (id, due) in [("anki", 1_792_616_400), ("amgi", last + 10 * 86_400), ("failed", last + 600)] {
            conn.execute(
                "INSERT INTO card_state VALUES (?1, 2, 5, ?2, ?3, 1, 0)",
                params![id, due, last],
            )
            .unwrap();
        }
        conn.pragma_update(None, "user_version", 5).unwrap();
        let states = Store::init(conn).unwrap().states().unwrap();
        assert_eq!(states["anki"].due, last + 10 * 86_400);
        assert_eq!(states["amgi"].due, last + 10 * 86_400);
        assert_eq!(states["failed"].due, last + 600);
    }
}
