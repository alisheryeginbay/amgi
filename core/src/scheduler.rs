//! Scheduling on top of the FSRS memory model from the `fsrs` crate
//! (https://github.com/open-spaced-repetition/fsrs-rs).
//!
//! The crate predicts memory state; this module adds the scheduling policy:
//! there are no learning steps, a failed card comes back after [`AGAIN_DELAY_SECS`],
//! and other intervals are whole days that increase strictly from Hard to Easy.

use fsrs::{FSRS, MemoryState};

pub const AGAIN_DELAY_SECS: i64 = 10 * 60;
const DAY_SECS: i64 = 86_400;
const MAX_INTERVAL_DAYS: f32 = 36_500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    pub const ALL: [Rating; 4] = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];

    /// From its number, 1 for Again to 4 for Easy.
    pub fn from_number(n: i64) -> Option<Rating> {
        Rating::ALL.get(usize::try_from(n).ok()?.checked_sub(1)?).copied()
    }
}

/// A card's stage when it was reviewed, numbered as in the FSRS optimizer's
/// review-log schema (https://github.com/open-spaced-repetition/fsrs-optimizer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    New = 0,
    Learning = 1,
    Review = 2,
    Relearning = 3,
}

impl ReviewState {
    pub fn from_number(n: i64) -> Option<ReviewState> {
        match n {
            0 => Some(ReviewState::New),
            1 => Some(ReviewState::Learning),
            2 => Some(ReviewState::Review),
            3 => Some(ReviewState::Relearning),
            _ => None,
        }
    }

    /// Stage of a review that follows one in stage `prev`, `elapsed_days` study days
    /// later. With no learning steps, a card is only seen again on the same day after
    /// Again: it is then still being learned, or relearned after a lapse.
    pub fn after(prev: ReviewState, elapsed_days: u32) -> ReviewState {
        match (elapsed_days, prev) {
            (1.., _) => ReviewState::Review,
            (0, ReviewState::New | ReviewState::Learning) => ReviewState::Learning,
            (0, ReviewState::Review | ReviewState::Relearning) => ReviewState::Relearning,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Memory {
    pub stability: f64,
    pub difficulty: f64,
}

impl From<Memory> for MemoryState {
    fn from(m: Memory) -> Self {
        MemoryState {
            stability: m.stability as f32,
            difficulty: m.difficulty as f32,
        }
    }
}

impl From<MemoryState> for Memory {
    fn from(m: MemoryState) -> Self {
        Memory {
            stability: m.stability.into(),
            difficulty: m.difficulty.into(),
        }
    }
}

/// Result of rating a card: its new memory state and when to show it next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub memory: Memory,
    pub interval_secs: i64,
}

/// Probability of recalling a card with this stability `elapsed_days` after its last
/// review: the forgetting curve of the default parameters, 90% at `stability` days.
pub fn retrievability(stability: f64, elapsed_days: f64) -> f64 {
    let state = MemoryState {
        stability: stability as f32,
        difficulty: 5.0, // The curve doesn't depend on difficulty.
    };
    fsrs::current_retrievability(state, elapsed_days.max(0.0) as f32, fsrs::DEFAULT_PARAMETERS[20]).into()
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    fsrs: FSRS,
    pub desired_retention: f32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler {
            fsrs: FSRS::default(),
            desired_retention: 0.9,
        }
    }
}

impl Scheduler {
    /// Outcomes for all four ratings, indexed by `rating as usize - 1`.
    /// `memory` is `None` for a card never reviewed; `elapsed_days` counts study days
    /// since the last review (0 for a review on the same day).
    pub fn preview(&self, memory: Option<Memory>, elapsed_days: u32) -> fsrs::Result<[Outcome; 4]> {
        let next = self
            .fsrs
            .next_states(memory.map(Into::into), self.desired_retention, elapsed_days)?;
        let states = [next.again, next.hard, next.good, next.easy];

        let mut prev_days = 0.0;
        Ok(Rating::ALL.map(|rating| {
            let state = &states[rating as usize - 1];
            let interval_secs = if rating == Rating::Again {
                AGAIN_DELAY_SECS
            } else {
                let days = state
                    .interval
                    .round()
                    .clamp(1.0, MAX_INTERVAL_DAYS)
                    .max(prev_days + 1.0);
                prev_days = days;
                days as i64 * DAY_SECS
            };
            Outcome {
                memory: state.memory.into(),
                interval_secs,
            }
        }))
    }

    /// Probability of recalling a card `elapsed_days` after its last review, on the
    /// forgetting curve of the default parameters the scheduler uses.
    pub fn retrievability(&self, memory: Memory, elapsed_days: f64) -> f64 {
        retrievability(memory.stability, elapsed_days)
    }

    pub fn schedule(
        &self,
        memory: Option<Memory>,
        elapsed_days: u32,
        rating: Rating,
    ) -> fsrs::Result<Outcome> {
        Ok(self.preview(memory, elapsed_days)?[rating as usize - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(o: &Outcome) -> i64 {
        o.interval_secs / DAY_SECS
    }

    #[test]
    fn retrievability_falls_to_retention_at_stability() {
        let s = Scheduler::default();
        let memory = Memory { stability: 10.0, difficulty: 5.0 };
        assert!((s.retrievability(memory, 0.0) - 1.0).abs() < 1e-6);
        assert!((s.retrievability(memory, 10.0) - 0.9).abs() < 1e-4);
        assert!(s.retrievability(memory, 30.0) < 0.9);
        assert_eq!(Rating::from_number(3), Some(Rating::Good));
        assert_eq!(Rating::from_number(0), None);
        assert_eq!(Rating::from_number(5), None);
    }

    #[test]
    fn review_states() {
        use ReviewState::*;
        assert_eq!([New, Learning, Review, Relearning].map(|s| ReviewState::from_number(s as i64)), [
            Some(New),
            Some(Learning),
            Some(Review),
            Some(Relearning)
        ]);
        assert_eq!(ReviewState::from_number(4), None);
        assert_eq!([New, Learning, Review, Relearning].map(|s| ReviewState::after(s, 0)), [
            Learning, Learning, Relearning, Relearning
        ]);
        assert_eq!([New, Learning, Relearning].map(|s| ReviewState::after(s, 3)), [Review; 3]);
    }

    #[test]
    fn new_card() {
        let s = Scheduler::default();
        let p = s.preview(None, 0).unwrap();
        assert_eq!(p[0].interval_secs, AGAIN_DELAY_SECS);
        assert_eq!([days(&p[1]), days(&p[2]), days(&p[3])], [1, 2, 8]);
        assert!((p[2].memory.stability - 2.3065).abs() < 1e-4);
        assert!(p[0].memory.difficulty > p[3].memory.difficulty);
    }

    #[test]
    fn successful_reviews_grow_intervals() {
        let s = Scheduler::default();
        let (mut memory, mut elapsed, mut last) = (None, 0, 0);
        for _ in 0..6 {
            let o = s.schedule(memory, elapsed, Rating::Good).unwrap();
            assert!(days(&o) > last, "{} <= {last}", days(&o));
            last = days(&o);
            elapsed = last as u32;
            memory = Some(o.memory);
        }
        assert!(last > 60);
    }

    #[test]
    fn lapse_shrinks_stability() {
        let s = Scheduler::default();
        let m = Memory {
            stability: 30.0,
            difficulty: 5.0,
        };
        let o = s.schedule(Some(m), 30, Rating::Again).unwrap();
        assert!(o.memory.stability < 30.0);
        assert!(o.memory.difficulty > 5.0);
    }

    #[test]
    fn same_day_review() {
        let s = Scheduler::default();
        let m = Memory {
            stability: 2.0,
            difficulty: 5.0,
        };
        let p = s.preview(Some(m), 0).unwrap();
        assert!(p[2].memory.stability >= 2.0);
        assert!(p[0].memory.stability < 2.0);
    }

    #[test]
    fn higher_retention_shortens_intervals() {
        let m = Memory {
            stability: 20.0,
            difficulty: 5.0,
        };
        let good = |desired_retention| {
            let s = Scheduler {
                desired_retention,
                ..Scheduler::default()
            };
            days(&s.schedule(Some(m), 20, Rating::Good).unwrap())
        };
        assert!(good(0.95) < good(0.9) && good(0.9) < good(0.8));
    }

    #[test]
    fn intervals_strictly_increase() {
        let s = Scheduler::default();
        let m = Memory {
            stability: 0.5,
            difficulty: 9.0,
        };
        let p = s.preview(Some(m), 0).unwrap();
        assert!(p[1].interval_secs < p[2].interval_secs && p[2].interval_secs < p[3].interval_secs);
    }
}
