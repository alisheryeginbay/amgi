//! Scheduling on top of the FSRS memory model from the `fsrs` crate
//! (https://github.com/open-spaced-repetition/fsrs-rs).
//!
//! The crate predicts memory state; this module adds the scheduling policy:
//! there are no learning steps, a failed card comes back after [`AGAIN_DELAY_SECS`],
//! and other intervals are whole days that increase strictly from Hard to Easy.
//!
//! The model comes in two versions, told apart by how many parameters it has: FSRS-7
//! with 34, the default, and FSRS-6 with 21, kept for parameters fitted to it. The
//! defaults were fitted to many people's reviews, or [`fit`] finds them for one
//! person's.

use fsrs::{FSRS, FSRSItem, FSRSReview, MemoryState};

pub const AGAIN_DELAY_SECS: i64 = 10 * 60;
const DAY_SECS: i64 = 86_400;
const MAX_INTERVAL_DAYS: f32 = 36_500.0;

/// How many parameters each version of the model has.
pub const FSRS6_PARAMETERS: usize = 21;
pub const FSRS7_PARAMETERS: usize = 34;

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

/// Time since a card's last review, in the two ways the model versions count it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Elapsed {
    /// Whole study days, 0 on the same day; what FSRS-6 goes by.
    pub days: u32,
    /// Seconds; what FSRS-7 goes by, as a fraction of days.
    pub secs: i64,
}

/// A card's memory state, as the model works it out after each review.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record, serde::Serialize)]
pub struct Memory {
    pub stability: f64,
    /// From 1 (easiest) to 10 (hardest).
    pub difficulty: f64,
    /// FSRS-7's stability of the part of a memory that fades fast; the same as
    /// `stability` under FSRS-6.
    pub stability_fast: f64,
}

impl Memory {
    /// A state with one stability, as FSRS-6 has.
    pub fn new(stability: f64, difficulty: f64) -> Memory {
        Memory { stability, difficulty, stability_fast: stability }
    }
}

impl From<Memory> for MemoryState {
    fn from(m: Memory) -> Self {
        MemoryState {
            stability: m.stability as f32,
            difficulty: m.difficulty as f32,
            stability_fast: m.stability_fast as f32,
        }
    }
}

impl From<MemoryState> for Memory {
    fn from(m: MemoryState) -> Self {
        Memory {
            stability: m.stability.into(),
            difficulty: m.difficulty.into(),
            stability_fast: m.stability_fast.into(),
        }
    }
}

/// Result of rating a card: its new memory state and when to show it next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub memory: Memory,
    pub interval_secs: i64,
}

/// The memory model's parameters: [`FSRS7_PARAMETERS`] or [`FSRS6_PARAMETERS`] of them.
pub type Parameters = Vec<f32>;

/// FSRS-7's default parameters.
pub fn default_parameters() -> Parameters {
    fsrs::DEFAULT_PARAMETERS.to_vec()
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    fsrs: FSRS,
    parameters: Parameters,
    pub desired_retention: f32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler::new(default_parameters(), 0.9).expect("the default parameters work")
    }
}

impl Scheduler {
    /// A scheduler with these parameters, or why they can't be used.
    pub fn new(parameters: Parameters, desired_retention: f32) -> fsrs::Result<Self> {
        let count = parameters.len();
        if !(count == FSRS6_PARAMETERS || count == FSRS7_PARAMETERS) || parameters.iter().any(|p| !p.is_finite()) {
            return Err(fsrs::FSRSError::InvalidParameters);
        }
        Ok(Scheduler {
            fsrs: FSRS::new(&parameters)?,
            parameters,
            desired_retention,
        })
    }

    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    fn is_fsrs7(&self) -> bool {
        self.parameters.len() == FSRS7_PARAMETERS
    }

    /// Days since the last review as this version of the model counts them.
    fn delta_t(&self, elapsed: Elapsed) -> f32 {
        if self.is_fsrs7() {
            elapsed.secs.max(0) as f32 / DAY_SECS as f32
        } else {
            elapsed.days as f32
        }
    }

    /// Outcomes for all four ratings, indexed by `rating as usize - 1`.
    /// `memory` is `None` for a card never reviewed, and `elapsed` is then ignored.
    pub fn preview(&self, memory: Option<Memory>, elapsed: Elapsed) -> fsrs::Result<[Outcome; 4]> {
        let next = self.fsrs.next_states_with_elapsed_days(
            memory.map(Into::into),
            self.desired_retention,
            self.delta_t(elapsed),
        )?;
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
    /// scheduler's forgetting curve.
    pub fn retrievability(&self, memory: Memory, elapsed_days: f64) -> f64 {
        self.fsrs.current_retrievability(memory.into(), elapsed_days.max(0.0) as f32).into()
    }

    /// Days after the review that left `memory` until recall falls to 90%. Under
    /// FSRS-6 that is the stability; FSRS-7's curve has more to it.
    pub fn days_to_90(&self, memory: Memory) -> f64 {
        if self.is_fsrs7() { self.fsrs.s90(memory.into()).into() } else { memory.stability }
    }

    /// The memory state after each of a card's reviews, from its first: the same as
    /// rating it with [`Self::schedule`] review by review.
    pub fn memories(&self, reviews: &[PastReview]) -> fsrs::Result<Vec<Memory>> {
        let states = self.fsrs.historical_memory_states(self.item(reviews), None)?;
        Ok(states.into_iter().map(Into::into).collect())
    }

    pub fn schedule(&self, memory: Option<Memory>, elapsed: Elapsed, rating: Rating) -> fsrs::Result<Outcome> {
        Ok(self.preview(memory, elapsed)?[rating as usize - 1])
    }

    /// One card's reviews as the crate takes them; the first is 0 days after nothing.
    fn item(&self, reviews: &[PastReview]) -> FSRSItem {
        FSRSItem {
            reviews: reviews
                .iter()
                .enumerate()
                .map(|(i, r)| FSRSReview {
                    rating: r.rating as u32,
                    delta_t: if i == 0 { 0.0 } else { self.delta_t(r.elapsed) },
                })
                .collect(),
        }
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
                stability: self.simulated_stability(c.memory) as f32,
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
                &self.parameters,
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

impl Scheduler {
    /// The stability to give the simulator for `memory`. Its FSRS-7 has one stability,
    /// as if the fast one were the same, so this is the one with which recall falls to
    /// 90% when the card's own does.
    fn simulated_stability(&self, memory: Memory) -> f64 {
        if !self.is_fsrs7() {
            return memory.stability;
        }
        let target = self.days_to_90(memory);
        let days_to_90 = |s: f64| self.days_to_90(Memory::new(s, 5.0));
        // Recall lasts longer the higher the stability; halve the range in log space.
        let (mut low, mut high) = (1e-3_f64.ln(), 1e5_f64.ln());
        for _ in 0..40 {
            let mid = (low + high) / 2.0;
            if days_to_90(mid.exp()) < target { low = mid } else { high = mid }
        }
        ((low + high) / 2.0).exp()
    }
}

/// A review in a card's history, for [`Scheduler::memories`] and [`fit`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PastReview {
    pub rating: Rating,
    /// Since the review before; ignored on the first.
    pub elapsed: Elapsed,
}

/// How well parameters predict whether reviews were remembered; lower is better.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Fitness {
    /// Average log loss of the predicted recall.
    pub log_loss: f64,
    /// How far predicted recall is from the share remembered, over reviews grouped
    /// alike: 0.05 is 5 points off.
    pub rmse: f64,
}

/// Result of [`fit`].
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub parameters: Parameters,
    /// Of the parameters in use, and of the fitted ones, on the same reviews.
    pub before: Fitness,
    pub after: Fitness,
    /// Reviews the fit learned to predict, each a day or more after the one before,
    /// and the cards they're of.
    pub reviews: usize,
    pub cards: usize,
}

/// FSRS-7 parameters fitted to `histories`, each a card's reviews from its very first,
/// compared with `current` on them; rounded to 4 decimals, as written down. `None`
/// without enough reviews on a later day than the one before to learn from.
pub fn fit(histories: &[Vec<PastReview>], current: &Parameters) -> fsrs::Result<Option<Fit>> {
    // An item per review on a later day than the one before, holding the reviews up
    // to it; same-day reviews are only history, as in Anki. Each version of the model
    // gets the items with time counted its way.
    let fitted_model = Scheduler::default();
    let current_model = Scheduler::new(current.clone(), 0.9)?;
    let (mut items, mut current_items, mut card_ids) = (Vec::new(), Vec::new(), Vec::new());
    for (id, history) in histories.iter().enumerate() {
        for end in 2..=history.len() {
            if history[end - 1].elapsed.days > 0 {
                items.push(fitted_model.item(&history[..end]));
                current_items.push(current_model.item(&history[..end]));
                card_ids.push(id as i64);
            }
        }
    }
    if items.is_empty() {
        return Ok(None);
    }
    let fitted = fsrs::compute_parameters(fsrs::ComputeParametersInput {
        train_set: items.clone(),
        card_ids: Some(card_ids.clone()),
        model_version: fsrs::ComputeParametersVersion::Fsrs7,
        // One more look after Again, as the card comes back ten minutes later.
        num_relearning_steps: Some(1),
        ..Default::default()
    })?;
    // What the crate gives back when there's too little to learn from.
    if fitted == fsrs::DEFAULT_PARAMETERS {
        return Ok(None);
    }
    let parameters: Parameters = fitted.iter().map(|p| (p * 10_000.0).round() / 10_000.0).collect();
    let fitness = |parameters: &Parameters, items: Vec<FSRSItem>| -> fsrs::Result<Fitness> {
        let e = FSRS::new(parameters)?.evaluate_with_card_ids(items, card_ids.clone(), |_| true)?;
        Ok(Fitness {
            log_loss: e.log_loss.into(),
            rmse: e.rmse_bins.into(),
        })
    };
    let mut cards = card_ids.clone();
    cards.dedup();
    Ok(Some(Fit {
        before: fitness(current, current_items)?,
        after: fitness(&parameters, items.clone())?,
        parameters,
        reviews: items.len(),
        cards: cards.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `days` study days after the last review, at the same time of day.
    fn after(days: u32) -> Elapsed {
        Elapsed { days, secs: i64::from(days) * DAY_SECS }
    }

    /// Back after Again's short delay.
    const SAME_DAY: Elapsed = Elapsed { days: 0, secs: AGAIN_DELAY_SECS };

    fn fsrs6() -> Scheduler {
        Scheduler::new(fsrs::FSRS6_DEFAULT_PARAMETERS.to_vec(), 0.9).unwrap()
    }

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
            memory: Memory::new(30.0, 5.0),
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
    fn retrievability_falls_to_90_percent_in_days_to_90() {
        let memory = Memory { stability: 10.0, difficulty: 5.0, stability_fast: 8.0 };
        for s in [Scheduler::default(), fsrs6()] {
            let days = s.days_to_90(memory);
            assert!((s.retrievability(memory, 0.0) - 1.0).abs() < 1e-4);
            assert!((s.retrievability(memory, days) - 0.9).abs() < 1e-3, "{days}");
            assert!(s.retrievability(memory, days * 3.0) < 0.9);
        }
        // FSRS-6's stability is just that.
        assert_eq!(fsrs6().days_to_90(memory), 10.0);
        assert_eq!(fsrs6().simulated_stability(memory), 10.0);
        // The simulator's FSRS-7 has one stability; it reaches 90% on the same day.
        let s = Scheduler::default();
        let simulated = Memory::new(s.simulated_stability(memory), 5.0);
        assert!((s.days_to_90(simulated) - s.days_to_90(memory)).abs() < 1e-2);
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
        let p = s.preview(None, Elapsed::default()).unwrap();
        assert_eq!(p[0].interval_secs, AGAIN_DELAY_SECS);
        assert_eq!([days(&p[1]), days(&p[2]), days(&p[3])], [1, 5, 54]);
        assert!((p[2].memory.stability - 3.9221).abs() < 1e-4);
        assert!(p[0].memory.difficulty > p[3].memory.difficulty);
    }

    #[test]
    fn fsrs6_schedules_as_before() {
        let p = fsrs6().preview(None, Elapsed::default()).unwrap();
        assert_eq!([days(&p[1]), days(&p[2]), days(&p[3])], [1, 2, 8]);
        assert!((p[2].memory.stability - 2.3065).abs() < 1e-4);
        assert_eq!(p[2].memory.stability_fast, p[2].memory.stability);
        // It counts whole study days, not the time of day.
        let m = Some(p[2].memory);
        let early = Elapsed { days: 2, secs: DAY_SECS };
        assert_eq!(fsrs6().preview(m, early).unwrap(), fsrs6().preview(m, after(2)).unwrap());
        assert_ne!(Scheduler::default().preview(m, early).unwrap(), Scheduler::default().preview(m, after(2)).unwrap());
        assert!(Scheduler::new(vec![1.0; 20], 0.9).is_err());
    }

    #[test]
    fn successful_reviews_grow_intervals() {
        let s = Scheduler::default();
        let (mut memory, mut elapsed, mut last) = (None, Elapsed::default(), 0);
        for _ in 0..6 {
            let o = s.schedule(memory, elapsed, Rating::Good).unwrap();
            assert!(days(&o) > last, "{} <= {last}", days(&o));
            last = days(&o);
            elapsed = after(last as u32);
            memory = Some(o.memory);
        }
        assert!(last > 60);
    }

    #[test]
    fn lapse_shrinks_stability() {
        let s = Scheduler::default();
        let m = Memory::new(30.0, 5.0);
        let o = s.schedule(Some(m), after(30), Rating::Again).unwrap();
        assert!(o.memory.stability < 30.0);
        assert!(o.memory.difficulty > 5.0);
    }

    #[test]
    fn same_day_review() {
        let s = Scheduler::default();
        let m = Memory::new(2.0, 5.0);
        let p = s.preview(Some(m), SAME_DAY).unwrap();
        assert!(p[2].memory.stability >= 2.0);
        assert!(p[0].memory.stability < 2.0);
    }

    #[test]
    fn higher_retention_shortens_intervals() {
        let m = Memory::new(20.0, 5.0);
        let good = |desired_retention| {
            let s = Scheduler {
                desired_retention,
                ..Scheduler::default()
            };
            days(&s.schedule(Some(m), after(20), Rating::Good).unwrap())
        };
        assert!(good(0.95) < good(0.9) && good(0.9) < good(0.8));
    }

    #[test]
    fn intervals_strictly_increase() {
        let s = Scheduler::default();
        let m = Memory::new(0.5, 9.0);
        let p = s.preview(Some(m), SAME_DAY).unwrap();
        assert!(p[1].interval_secs < p[2].interval_secs && p[2].interval_secs < p[3].interval_secs);
    }

    /// Reviews of `cards` cards by someone who forgets as `truth` says, each due when
    /// the scheduler has it, give or take a few days.
    fn simulated_histories(truth: &Parameters, cards: usize) -> Vec<Vec<PastReview>> {
        let s = Scheduler::new(truth.clone(), 0.9).unwrap();
        let mut seed: u64 = 42;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        (0..cards)
            .map(|_| {
                let mut history = vec![PastReview { rating: Rating::Good, elapsed: Elapsed::default() }];
                let mut memory = s.schedule(None, Elapsed::default(), Rating::Good).unwrap().memory;
                for _ in 0..8 {
                    let interval = s.schedule(Some(memory), SAME_DAY, Rating::Good).unwrap().interval_secs / DAY_SECS;
                    let days = ((interval as f64 * (0.7 + 0.6 * random())) as u32).max(1);
                    let remembered = random() < s.retrievability(memory, days.into());
                    let rating = if remembered { Rating::Good } else { Rating::Again };
                    memory = s.schedule(Some(memory), after(days), rating).unwrap().memory;
                    history.push(PastReview { rating, elapsed: after(days) });
                    if !remembered {
                        // Seen again ten minutes later.
                        memory = s.schedule(Some(memory), SAME_DAY, Rating::Good).unwrap().memory;
                        history.push(PastReview { rating: Rating::Good, elapsed: SAME_DAY });
                    }
                }
                history
            })
            .collect()
    }

    #[test]
    fn fits_someone_who_remembers_longer() {
        let mut truth = default_parameters();
        for p in &mut truth[..4] {
            *p *= 3.0;
        }
        let histories = simulated_histories(&truth, 400);
        let fit = fit(&histories, &default_parameters()).unwrap().unwrap();
        // One review a day or more after the one before for each of the 8 rounds.
        assert_eq!((fit.reviews, fit.cards), (400 * 8, 400));
        assert!(fit.after.log_loss < fit.before.log_loss, "{:?} vs {:?}", fit.after, fit.before);
        assert!(fit.after.rmse < fit.before.rmse, "{:?} vs {:?}", fit.after, fit.before);
        // Good on a first review lasts longer than the defaults say.
        assert!(fit.parameters[2] > fsrs::DEFAULT_PARAMETERS[2], "{:?}", fit.parameters);
        assert!(fit.parameters.iter().all(|p| (p * 10_000.0).round() / 10_000.0 == *p));
        assert!(Scheduler::new(fit.parameters, 0.9).is_ok());
    }

    #[test]
    fn fits_fsrs7_in_place_of_fsrs6() {
        let fsrs6 = fsrs::FSRS6_DEFAULT_PARAMETERS.to_vec();
        let histories = simulated_histories(&default_parameters(), 300);
        let fit = fit(&histories, &fsrs6).unwrap().unwrap();
        assert_eq!(fit.parameters.len(), FSRS7_PARAMETERS);
        assert!(fit.after.log_loss < fit.before.log_loss, "{:?} vs {:?}", fit.after, fit.before);
    }

    #[test]
    fn too_few_reviews_to_fit() {
        let truth = default_parameters();
        assert_eq!(fit(&simulated_histories(&truth, 1), &truth).unwrap(), None);
        // Only same-day reviews: nothing to predict.
        let same_day = vec![vec![PastReview { rating: Rating::Again, elapsed: Elapsed::default() }, PastReview { rating: Rating::Good, elapsed: SAME_DAY }]; 50];
        assert_eq!(fit(&same_day, &truth).unwrap(), None);
    }
}
