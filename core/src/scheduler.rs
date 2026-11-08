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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum, serde::Serialize)]
#[serde(rename_all = "lowercase")]
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

/// A card already studied, for [`Scheduler::simulate_reviews`], with days counted from
/// today.
#[derive(Debug, Clone, Copy)]
pub struct PlannedCard {
    pub memory: Memory,
    /// Day of the last review: 0 or before.
    pub last_day: i64,
    /// Day it comes due: 0 for today or overdue.
    pub due_day: i64,
    pub lapses: u32,
}

/// How often each rating is given, for [`Scheduler::simulate_reviews`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatingHabits {
    /// On a card's first review, Again to Easy.
    pub first: [f32; 4],
    /// On a remembered review, Hard to Easy.
    pub passed: [f32; 3],
}

impl Default for RatingHabits {
    /// The FSRS simulator's defaults, taken from many people's reviews.
    fn default() -> Self {
        let config = fsrs::SimulatorConfig::default();
        RatingHabits {
            first: config.first_rating_prob,
            passed: config.review_rating_prob,
        }
    }
}

/// Simulated runs averaged in [`Scheduler::simulate_reviews`].
const SIMULATION_RUNS: u64 = 50;

impl Scheduler {
    /// Reviews expected on each of `days` days from today, of `cards` and of `new_cards`
    /// first studied up to `per_day` a day from today, not counting those first views.
    /// It averages runs of the
    /// FSRS simulator, which draws whether each review is remembered from the card's
    /// retrievability and its rating from `habits`. A failed card is studied again the
    /// same day, as with Again's short delay. There's no daily review limit; a card
    /// stops coming back after `suspend_after` lapses, when that is set.
    pub fn simulate_reviews(
        &self,
        cards: &[PlannedCard],
        new_cards: u32,
        per_day: u32,
        days: usize,
        habits: RatingHabits,
        suspend_after: Option<u32>,
    ) -> fsrs::Result<Vec<f64>> {
        let deck_size = cards.len() + new_cards as usize;
        if deck_size == 0 || days == 0 {
            return Ok(vec![0.0; days]);
        }
        let config = fsrs::SimulatorConfig {
            deck_size,
            learn_span: days,
            max_cost_perday: f32::INFINITY,
            max_ivl: MAX_INTERVAL_DAYS,
            first_rating_prob: habits.first,
            review_rating_prob: habits.passed,
            learn_limit: per_day as usize,
            suspend_after_lapses: suspend_after,
            // One more look after Again, as the card comes back ten minutes later.
            learning_step_count: 1,
            relearning_step_count: 1,
            ..fsrs::SimulatorConfig::default()
        };
        let existing: Vec<fsrs::Card> = cards
            .iter()
            .enumerate()
            .map(|(i, c)| fsrs::Card {
                id: i as i64 + 1,
                difficulty: c.memory.difficulty as f32,
                stability: c.memory.stability as f32,
                last_date: c.last_day as f32,
                due: c.due_day.max(0) as f32,
                interval: (c.due_day - c.last_day).max(1) as f32,
                lapses: c.lapses,
                desired_retention: self.desired_retention,
                ..fsrs::Card::default()
            })
            .collect();
        let mut totals = vec![0usize; days];
        for seed in 0..SIMULATION_RUNS {
            let result = fsrs::simulate(
                &config,
                &fsrs::DEFAULT_PARAMETERS,
                self.desired_retention,
                Some(seed),
                Some(existing.clone()),
            )?;
            for (total, n) in totals.iter_mut().zip(result.review_cnt_per_day) {
                *total += n;
            }
        }
        Ok(totals.into_iter().map(|n| n as f64 / SIMULATION_RUNS as f64).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_reviews_of_new_cards() {
        let s = Scheduler::default();
        let habits = RatingHabits::default();
        let reviews = s.simulate_reviews(&[], 100, 100, 60, habits, None).unwrap();
        // First views aren't counted, and most cards come back within days.
        assert_eq!(reviews[0], 0.0);
        assert!(reviews[1..8].iter().sum::<f64>() > 80.0);
        // Fewer each week as intervals grow.
        let week = |w: usize| reviews[w * 7..w * 7 + 7].iter().sum::<f64>();
        assert!(week(1) > week(6), "{} <= {}", week(1), week(6));
        assert_eq!(s.simulate_reviews(&[], 0, 0, 30, habits, None).unwrap(), vec![0.0; 30]);
    }

    #[test]
    fn simulated_reviews_of_studied_cards() {
        let s = Scheduler::default();
        let card = |due_day| PlannedCard {
            memory: Memory { stability: 30.0, difficulty: 5.0 },
            last_day: -20,
            due_day,
            lapses: 0,
        };
        let reviews = s.simulate_reviews(&[card(-3), card(0), card(4)], 0, 0, 10, RatingHabits::default(), None).unwrap();
        // Overdue cards are due today; a remembered card isn't back within the 10 days,
        // but a forgotten one can be.
        assert_eq!(reviews[0], 2.0);
        assert!((1.0..1.2).contains(&reviews[4]), "{}", reviews[4]);
        assert!(reviews.iter().sum::<f64>() < 3.5);
    }

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
