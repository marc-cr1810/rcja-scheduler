//! Unified conflict model shared by the solver and the GUI.
//!
//! Every hard and soft rule is evaluated in exactly one place
//! ([`super::fast_evaluator::FastEvaluator`]). That single pass reports each
//! violation it finds to a [`ConflictSink`]. Two sinks consume those reports:
//!
//! * [`ScalarSink`] — the solver's hot path. It only sums weights into a
//!   `(hard, soft)` cost and allocates nothing, so the rule body monomorphizes
//!   down to plain `hard += w` / `soft += w`.
//! * [`RecordSink`] — the display / diagnostics path. It keeps a structured
//!   [`Conflict`] per violation, from which we derive the conflict count, the
//!   conflicted-assignment indices used to bias mutation, and the
//!   human-readable messages shown in the UI.
//!
//! Because cost and conflicts come from the same code, the numbers can never
//! drift apart the way the old parallel implementations did.

/// Whether a violation counts against the hard-conflict budget (must be zero
/// for a valid schedule) or the soft-penalty budget (optimised, but tolerated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CostClass {
    Hard,
    Soft,
}

/// The specific kind of violation, carrying the internal indices needed to
/// format a human-readable message. Aggregate/statistical penalties (variance,
/// fairness, …) have no single assignment to point at and are not surfaced as
/// per-assignment conflicts; they exist here only so their cost flows through
/// the same sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    // --- Hard, attributable to specific assignment(s) ---
    /// An interview placed in a competition slot, or vice-versa.
    SlotKindMismatch,
    /// The assigned field can't host this activity (kind/division mismatch).
    FieldUnsuitable { field_idx: usize },
    /// No field assigned at all.
    FieldMissing,
    /// A volunteer is rostered during a slot they're not available for.
    VolUnavailable { vol_idx: usize, slot_idx: usize },
    /// A volunteer lacks the capability required for this activity (hard when
    /// strict capabilities are on, or for interviews).
    VolUnqualified { vol_idx: usize },
    /// A volunteer has a declared conflict of interest with a participating team.
    ConflictOfInterest { vol_idx: usize, team_idx: usize },
    /// Fewer volunteers rostered than the division requires.
    UnderRostered { required: usize, assigned: usize },
    /// An interview scheduled on a day where interviews are disabled.
    InterviewsDisabled,
    /// The activity runs past the end of its day.
    DurationExceedsDay,
    /// A volunteer exceeds the per-day shift cap (cost only, not displayed).
    DailyShiftCapExceeded { vol_idx: usize },
    /// Two activities share a team in overlapping time.
    TeamDoubleBooked { team_idx: usize },
    /// Two activities share a field in overlapping time.
    FieldDoubleBooked { field_idx: usize },
    /// Two activities share a volunteer in overlapping time.
    VolDoubleBooked { vol_idx: usize },
    /// A later-stage match (e.g. finals) is scheduled before an earlier stage.
    StageOrder,
    /// An earlier-stage match overlaps a later stage in time.
    StageOverlap,
    /// Strict field-variety: a team is assigned the same field more than once.
    FieldVarietyStrict { team_idx: usize, field_idx: usize },

    // --- Soft, attributable ---
    /// A volunteer lacks a non-strict division capability (soft penalty).
    VolCapabilitySoft { vol_idx: usize },
    /// Interview scheduled late in the day.
    InterviewLate,
    /// Excessive wait time between a team's matches.
    TeamWaitTime,
    /// A team plays back-to-back with no break.
    TeamBackToBack,
    /// A team's interview is too close to one of its matches.
    InterviewMatchGap,
    /// A volunteer works consecutive slots.
    VolConsecutive,
    /// A volunteer travels between fields on consecutive slots.
    VolTravel,
    /// Rounds within a division run out of chronological order.
    RoundOrder,
    /// Non-strict field-variety repeat.
    FieldVariety,

    // --- Soft, aggregate (cost only; no per-assignment message) ---
    /// Variance in per-field match/interview load.
    FieldBalance,
    /// Variance in volunteer utilisation (fairness).
    VolFairness,
    /// A volunteer spread across too many divisions (specialist mode).
    Specialist,
    /// Uneven activity spread across time slots (peak-period smoothing).
    PeakPeriod,
}

/// One reported violation. `who` lists the assignment indices it involves
/// (empty for aggregate penalties that don't map to a single assignment).
#[derive(Debug, Clone)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub class: CostClass,
    /// This violation's contribution to the scalar cost. Kept on the record so
    /// cost and conflicts stay derivable from one another (asserted in tests);
    /// the display path doesn't read it.
    #[allow(dead_code)]
    pub weight: f64,
    pub who: Vec<usize>,
}

/// The distinct hard conflicts in `records`, deduplicated by `(kind, who)`.
///
/// The engine reports occupancy conflicts once per overlapping time bucket, so a
/// single double-booking can surface as several identical records; collapsing
/// them here gives one canonical entry per real problem. This is the single
/// definition of "a hard conflict", so the headline count and the diagnostics
/// list always agree.
pub fn distinct_hard_conflicts(records: &[Conflict]) -> Vec<Conflict> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in records {
        if c.class == CostClass::Hard && seen.insert((c.kind, c.who.clone())) {
            out.push(c.clone());
        }
    }
    out
}

/// Receives every violation the rule engine finds. Implemented once per output
/// shape; the rule body is written against `&mut impl ConflictSink`.
pub trait ConflictSink {
    fn report(&mut self, class: CostClass, weight: f64, kind: ConflictKind, who: &[usize]);

    /// Whether this sink needs soft penalties computed. Sinks that only care
    /// about hard conflicts (e.g. mutation targeting) return `false` so the
    /// engine can skip the expensive soft-penalty passes.
    fn wants_soft(&self) -> bool {
        true
    }
}

/// Hot-path sink: accumulates only the scalar `(hard, soft)` cost. Allocates
/// nothing — `report` inlines to a single add.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScalarSink {
    pub hard: f64,
    pub soft: f64,
}

impl ConflictSink for ScalarSink {
    #[inline]
    fn report(&mut self, class: CostClass, weight: f64, _kind: ConflictKind, _who: &[usize]) {
        match class {
            CostClass::Hard => self.hard += weight,
            CostClass::Soft => self.soft += weight,
        }
    }
}

/// Display/diagnostics sink: keeps a structured record per violation.
#[derive(Debug, Clone, Default)]
pub struct RecordSink {
    pub records: Vec<Conflict>,
}

impl ConflictSink for RecordSink {
    fn report(&mut self, class: CostClass, weight: f64, kind: ConflictKind, who: &[usize]) {
        self.records.push(Conflict {
            kind,
            class,
            weight,
            who: who.to_vec(),
        });
    }
}

/// Lightweight sink for mutation targeting: records only which assignments are
/// involved in a hard conflict, with no per-conflict allocation. Skips soft
/// penalties entirely.
#[derive(Debug, Clone, Default)]
pub struct ConflictedSink {
    conflicted: Vec<bool>,
}

impl ConflictedSink {
    pub fn new(num_assignments: usize) -> Self {
        Self { conflicted: vec![false; num_assignments] }
    }

    pub fn into_indices(self) -> Vec<usize> {
        self.conflicted
            .iter()
            .enumerate()
            .filter_map(|(i, &c)| if c { Some(i) } else { None })
            .collect()
    }
}

impl ConflictSink for ConflictedSink {
    #[inline]
    fn report(&mut self, class: CostClass, _weight: f64, _kind: ConflictKind, who: &[usize]) {
        if class == CostClass::Hard {
            for &i in who {
                if i < self.conflicted.len() {
                    self.conflicted[i] = true;
                }
            }
        }
    }

    fn wants_soft(&self) -> bool {
        false
    }
}
