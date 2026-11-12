//! SQLite persistence. Card content lives in Markdown; the database only holds
//! scheduling state and the review log, keyed by content-derived card ids.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};

use crate::scheduler::{Elapsed, Memory, Outcome, Rating, ReviewState};

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
-- No longer used: tag icons and saved views went with the app's browse pane.
CREATE TABLE tag_icon (
    tag  TEXT PRIMARY KEY,
    icon TEXT NOT NULL
);
"#, r#"
-- No longer used: tag icons and saved views went with the app's browse pane.
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
"#, r#"
-- Milliseconds spent on the card before rating it; NULL when not known, as for
-- reviews logged before this column.
ALTER TABLE review_log ADD COLUMN taken_ms INTEGER;
"#, r#"
-- FSRS-7's stability of the part of a memory that fades fast; NULL for states worked
-- out by FSRS-6, whose memories have one stability.
ALTER TABLE card_state ADD COLUMN stability_fast REAL;
ALTER TABLE review_log ADD COLUMN stability_fast REAL;
"#];

/// The `stability_fast` column, the same as `stability` where FSRS-6 left it out.
const STABILITY_FAST: &str = "COALESCE(stability_fast, stability)";

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
    /// Milliseconds spent on the card before rating it; `None` when not known.
    pub taken_ms: Option<u32>,
}

/// A review in [`Store::histories`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryRow {
    /// Its row in the log.
    pub id: i64,
    pub rating: Rating,
    /// Since the review before.
    pub elapsed: Elapsed,
    /// Whether it was the card's first review, not one partway through a history
    /// imported without its start.
    pub first: bool,
}

/// An entry to write to the review log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogEntry {
    Review(LoggedReview),
    /// The card was reset to new at this time (Unix seconds).
    Reset(i64),
}

/// Average milliseconds spent, from `Store::time_per_card`. `None` without timed reviews.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CardTime {
    /// On a new card the day it was first studied.
    pub new_ms: Option<f64>,
    /// On a card studied before, the day it came back.
    pub seen_ms: Option<f64>,
    /// On one review.
    pub review_ms: Option<f64>,
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
            "SELECT card_id, stability, difficulty, due, last_review, reps, lapses, {LAST_INTERVAL}, {STABILITY_FAST}
             FROM card_state"
        ))?;
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                CardState {
                    memory: Memory {
                        stability: r.get(1)?,
                        difficulty: r.get(2)?,
                        stability_fast: r.get(8)?,
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
                    "SELECT stability, difficulty, due, last_review, reps, lapses, {LAST_INTERVAL}, {STABILITY_FAST}
                     FROM card_state WHERE card_id = ?1"
                ),
                [card_id],
                |r| {
                    Ok(CardState {
                        memory: Memory {
                            stability: r.get(0)?,
                            difficulty: r.get(1)?,
                            stability_fast: r.get(7)?,
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
        let mut stmt = self.conn.prepare(&format!(
            "SELECT reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new, review_state, taken_ms,
               {STABILITY_FAST}
             FROM review_log WHERE card_id = ?1 ORDER BY reviewed_at, id",
        ))?;
        let rows = stmt.query_map([card_id], |r| {
            let Some(rating) = Rating::from_number(r.get(1)?) else { return Ok(None) };
            Ok(Some(LoggedReview {
                reviewed_at: r.get(0)?,
                rating,
                elapsed_days: r.get(2)?,
                memory: Memory {
                    stability: r.get(3)?,
                    difficulty: r.get(4)?,
                    stability_fast: r.get(9)?,
                },
                interval_secs: r.get(5)?,
                was_new: r.get(6)?,
                state: r.get::<_, Option<i64>>(7)?.and_then(ReviewState::from_number),
                taken_ms: r.get(8)?,
            }))
        })?;
        rows.filter_map(Result::transpose).collect()
    }

    /// Every card's reviews since it was last reset to new, oldest first, by card id;
    /// cards with none are left out.
    pub fn histories(&self) -> rusqlite::Result<Vec<(String, Vec<HistoryRow>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT card_id, id, rating, elapsed_days, was_new, review_state, reviewed_at
             FROM review_log ORDER BY card_id, reviewed_at, id",
        )?;
        let mut rows = stmt.query([])?;
        let mut cards: Vec<(String, Vec<HistoryRow>)> = Vec::new();
        // When the review before was, on the same card.
        let mut prev_at = 0;
        while let Some(r) = rows.next()? {
            let card_id: String = r.get(0)?;
            let reviewed_at: i64 = r.get(6)?;
            if cards.last().is_none_or(|(id, _)| *id != card_id) {
                cards.push((card_id, Vec::new()));
                prev_at = reviewed_at;
            }
            let secs = reviewed_at - std::mem::replace(&mut prev_at, reviewed_at);
            let history = &mut cards.last_mut().expect("just pushed").1;
            let Some(rating) = Rating::from_number(r.get(2)?) else {
                // A reset, or a rating not known.
                history.clear();
                continue;
            };
            let state = r.get::<_, Option<i64>>(5)?.and_then(ReviewState::from_number);
            history.push(HistoryRow {
                id: r.get(1)?,
                rating,
                elapsed: Elapsed {
                    days: r.get::<_, f64>(3)?.round().max(0.0) as u32,
                    secs: if history.is_empty() { 0 } else { secs },
                },
                // Before the state was logged, the first review of a new card.
                first: state.map_or(r.get(4)?, |s| s == ReviewState::New),
            });
        }
        cards.retain(|(_, h)| !h.is_empty());
        Ok(cards)
    }

    /// Write memory states recalculated from the review log: those after each review,
    /// by log row, and each card's latest. Due dates stay as they were.
    pub fn set_memories(&mut self, rows: &[(i64, Memory)], cards: &[(String, Memory)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut row = tx.prepare(
                "UPDATE review_log SET stability = ?2, difficulty = ?3, stability_fast = ?4 WHERE id = ?1",
            )?;
            for (id, m) in rows {
                row.execute(params![id, m.stability, m.difficulty, m.stability_fast])?;
            }
            let mut card = tx.prepare(
                "UPDATE card_state SET stability = ?2, difficulty = ?3, stability_fast = ?4 WHERE card_id = ?1",
            )?;
            for (id, m) in cards {
                card.execute(params![id, m.stability, m.difficulty, m.stability_fast])?;
            }
        }
        tx.commit()
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
        taken_ms: Option<u32>,
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
                taken_ms,
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

    /// Ids of the new cards first reviewed at or after `since`.
    pub fn new_card_ids_since(&self, since: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT card_id FROM review_log WHERE was_new AND reviewed_at >= ?1")?;
        let ids = stmt.query_map([since], |r| r.get(0))?;
        ids.collect()
    }

    /// How often each rating was given: on a card's first review, Again to Easy, and on
    /// remembered reviews of cards due after a day or more, Hard to Easy.
    pub fn rating_counts(&self) -> rusqlite::Result<([u32; 4], [u32; 3])> {
        let mut stmt = self.conn.prepare(
            "SELECT was_new, rating, COUNT(*) FROM review_log
             WHERE rating > 0 AND (was_new OR (review_state = ?1 AND rating > 1))
             GROUP BY was_new, rating",
        )?;
        let (mut first, mut passed) = ([0; 4], [0; 3]);
        let rows = stmt.query_map([ReviewState::Review as i64], |r| {
            Ok((r.get::<_, bool>(0)?, r.get::<_, i64>(1)?, r.get::<_, u32>(2)?))
        })?;
        for row in rows {
            match row? {
                (true, rating @ 1..=4, n) => first[rating as usize - 1] += n,
                (false, rating @ 2..=4, n) => passed[rating as usize - 2] += n,
                _ => {}
            }
        }
        Ok((first, passed))
    }

    /// Reviews per day at or after `since`, where day `n` starts at `n * 86400 - shift`
    /// seconds: `(day, count, milliseconds spent)`, counting only the reviews timed.
    /// Days without reviews are absent.
    pub fn reviews_per_day(
        &self,
        since: i64,
        shift: i64,
        cards: Option<&[String]>,
    ) -> rusqlite::Result<Vec<(i64, u32, i64)>> {
        let (all, ids) = card_list(cards);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT (reviewed_at + :shift) / 86400 AS day, COUNT(*), COALESCE(SUM(taken_ms), 0) FROM review_log
             WHERE rating > 0 AND reviewed_at >= :since AND {IN_CARDS} GROUP BY day ORDER BY day"
        ))?;
        stmt.query_map(named_params! { ":since": since, ":shift": shift, ":all": all, ":cards": ids }, |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect()
    }

    /// Milliseconds spent on reviews at or after `since`, counting only the reviews timed.
    pub fn time_since(&self, since: i64, cards: Option<&[String]>) -> rusqlite::Result<i64> {
        let (all, ids) = card_list(cards);
        self.conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(taken_ms), 0) FROM review_log
                 WHERE rating > 0 AND reviewed_at >= :since AND {IN_CARDS}"
            ),
            named_params! { ":since": since, ":all": all, ":cards": ids },
            |r| r.get(0),
        )
    }

    /// Average time spent on a card on a day it was studied, at or after `since`, with
    /// days as in `reviews_per_day`: all of that day's reviews of it, such as again
    /// after forgetting it. Only timed reviews count.
    pub fn time_per_card(&self, since: i64, shift: i64, cards: Option<&[String]>) -> rusqlite::Result<CardTime> {
        let (all, ids) = card_list(cards);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT new, AVG(ms), SUM(ms), SUM(reviews) FROM
               (SELECT MAX(was_new) AS new, SUM(taken_ms) AS ms, COUNT(*) AS reviews FROM review_log
                WHERE rating > 0 AND taken_ms IS NOT NULL AND reviewed_at >= :since AND {IN_CARDS}
                GROUP BY card_id, (reviewed_at + :shift) / 86400)
             GROUP BY new"
        ))?;
        let mut time = CardTime::default();
        let (mut ms, mut reviews) = (0.0, 0.0);
        let rows = stmt.query_map(named_params! { ":since": since, ":shift": shift, ":all": all, ":cards": ids }, |r| {
            Ok((r.get::<_, bool>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?))
        })?;
        for row in rows {
            let (new, average, total, count) = row?;
            *(if new { &mut time.new_ms } else { &mut time.seen_ms }) = Some(average);
            ms += total;
            reviews += count;
        }
        time.review_ms = (reviews > 0.0).then(|| ms / reviews);
        Ok(time)
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
}

fn insert_review(tx: &Transaction, card_id: &str, r: &LoggedReview) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO review_log
           (card_id, reviewed_at, rating, elapsed_days, stability, difficulty, interval_secs, was_new, review_state, taken_ms,
            stability_fast)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            r.taken_ms,
            r.memory.stability_fast,
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
        "INSERT INTO card_state (card_id, stability, difficulty, due, last_review, reps, lapses, stability_fast)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (card_id) DO UPDATE SET
           stability = excluded.stability, difficulty = excluded.difficulty, stability_fast = excluded.stability_fast,
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
            state.memory.stability_fast,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_reload() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = Outcome {
            memory: Memory { stability: 2.0, difficulty: 5.0, stability_fast: 1.5 },
            interval_secs: 86_400,
        };
        let (s1, _) = store
            .record_review("abc", 1000, Rating::Good, 0.0, ReviewState::New, Some(4000), None, outcome)
            .unwrap();
        assert_eq!((s1.reps, s1.lapses, s1.due), (1, 0, 87_400));

        let (s2, _) = store
            .record_review("abc", 90_000, Rating::Again, 1.0, ReviewState::Review, Some(2500), Some(s1), outcome)
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
    fn review_time() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = Outcome { memory: Memory::new(2.0, 5.0), interval_secs: 600 };
        // A new card, failed and passed on day 0; a seen one on day 1; one not timed.
        let (a, _) = store
            .record_review("a", 1000, Rating::Again, 0.0, ReviewState::New, Some(4000), None, outcome)
            .unwrap();
        store
            .record_review("a", 1600, Rating::Good, 0.0, ReviewState::Learning, Some(2000), Some(a), outcome)
            .unwrap();
        store
            .record_review("b", 90_000, Rating::Good, 1.0, ReviewState::Review, Some(3000), Some(a), outcome)
            .unwrap();
        store
            .record_review("c", 90_100, Rating::Good, 0.0, ReviewState::New, None, None, outcome)
            .unwrap();

        assert_eq!(store.reviews("a").unwrap().iter().map(|r| r.taken_ms).collect::<Vec<_>>(), [Some(4000), Some(2000)]);
        assert_eq!(store.reviews_per_day(0, 0, None).unwrap(), [(0, 2, 6000), (1, 2, 3000)]);
        assert_eq!(store.time_since(86_400, None).unwrap(), 3000);
        assert_eq!(store.time_since(0, Some(&["a".into()])).unwrap(), 6000);
        let time = store.time_per_card(0, 0, None).unwrap();
        assert_eq!(time, CardTime { new_ms: Some(6000.0), seen_ms: Some(3000.0), review_ms: Some(3000.0) });
        assert_eq!(store.time_per_card(86_400, 0, Some(&["c".into()])).unwrap(), CardTime::default());
    }

    #[test]
    fn undo_reviews() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = |interval_secs| Outcome { memory: Memory::new(2.0, 5.0), interval_secs };
        let (s1, r1) = store
            .record_review("abc", 1000, Rating::Good, 0.0, ReviewState::New, None, None, outcome(86_400))
            .unwrap();
        let (_, r2) = store
            .record_review("abc", 90_000, Rating::Again, 1.0, ReviewState::Review, None, Some(s1), outcome(600))
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
            memory: Memory { stability: 2.0, difficulty: 5.0, stability_fast: 1.5 },
            interval_secs: 86_400,
            was_new: true,
            state: Some(ReviewState::New),
            taken_ms: Some(1500),
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
        assert_eq!(store.reviews_per_day(0, 0, None).unwrap(), [(0, 2, 3000)]);
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
        let review = store.reviews("abc").unwrap()[0];
        assert_eq!(review.state, None);
        // FSRS-6 left one stability.
        assert_eq!(review.memory, Memory::new(2.0, 5.0));
        assert_eq!(store.last_review_state("abc").unwrap(), None);
    }

    #[test]
    fn histories_count_days_and_seconds() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = Outcome { memory: Memory::new(2.0, 5.0), interval_secs: 600 };
        let (s1, _) = store.record_review("a", 1000, Rating::Again, 0.0, ReviewState::New, None, None, outcome).unwrap();
        let (s2, _) = store.record_review("a", 1600, Rating::Good, 0.0, ReviewState::Learning, None, Some(s1), outcome).unwrap();
        store.record_review("a", 200_000, Rating::Good, 2.0, ReviewState::Review, None, Some(s2), outcome).unwrap();
        store.record_review("b", 5000, Rating::Good, 0.0, ReviewState::New, None, None, outcome).unwrap();
        let histories = store.histories().unwrap();
        let elapsed = |card: usize| histories[card].1.iter().map(|r| (r.elapsed.days, r.elapsed.secs)).collect::<Vec<_>>();
        assert_eq!(elapsed(0), [(0, 0), (0, 600), (2, 198_400)]);
        assert_eq!(elapsed(1), [(0, 0)]);
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
