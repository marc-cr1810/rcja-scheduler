use super::internal::{InternalTournamentConfig, InternalAssignment, InternalSchedule};
use super::SolverParams;
use super::conflicts::{Conflict, ConflictKind, ConflictSink, ConflictedSink, CostClass, RecordSink, ScalarSink};
use crate::model::{Activity, FairnessMode, SpecialistMode, FieldKind, Schedule, ScheduleAssignment, SchedulingMode, TournamentConfig};
use std::collections::HashMap;

pub struct FastEvaluator<'a> {
    config: &'a InternalTournamentConfig,
    params: &'a SolverParams,
    
    // Hard Conflict State
    team_slot_occupancy: Vec<Vec<u32>>, 
    field_slot_occupancy: Vec<Vec<u32>>, 
    volunteer_slot_occupancy: Vec<Vec<u32>>, 
    volunteer_daily_counts: Vec<Vec<u32>>,
    
    // Load tracking (for variance and individual penalties)
    volunteer_shift_counts: Vec<u32>,
    field_match_counts: Vec<u32>,
    field_interview_counts: Vec<u32>,
    field_total_counts: Vec<u32>,

    // Grouped assignment tracking (for back-to-back, wait time, travel)
    // We keep sorted lists of (slot_idx, activity_idx) for each team/volunteer/day
    team_day_assignments: Vec<Vec<Vec<usize>>>, // [team][day] -> sorted list of assignment indices
    vol_day_assignments: Vec<Vec<Vec<usize>>>, // [vol][day] -> sorted list of assignment indices
    division_assignments: Vec<Vec<usize>>, // [division] -> sorted list of assignment indices

    // Current total cost
    current_hard_conflicts: f64,
    current_soft_penalties: f64,

    // --- Incremental (delta) evaluation state ---
    // Set up by `inc_init` and maintained across mutations by `apply_delta` /
    // `revert_delta`, so the solver can re-score a single move in O(touched)
    // instead of re-scanning the whole schedule. Every running total below is an
    // exact mirror of what the full `evaluate` would sum; the guardrail test
    // `incremental_matches_full_recompute` asserts this on every step.
    inc_ready: bool,
    /// Per-assignment local (hard, soft) cost, and its running sum.
    inc_local: Vec<(f64, f64)>,
    inc_sum_local: (f64, f64),
    /// Occupancy double-booking + daily-shift-cap penalty (all hard).
    inc_occ_hard: f64,
    /// Per-(team, day) break penalty (hard, soft) and running sum.
    inc_team_day: Vec<Vec<(f64, f64)>>,
    inc_sum_team_day: (f64, f64),
    /// Per-(vol, day) consecutive/travel penalty (soft) and running sum.
    inc_vol_day: Vec<Vec<f64>>,
    inc_sum_vol_day: f64,
    /// Per-division stage-order/round-order penalty (hard, soft) and running sum.
    inc_div: Vec<(f64, f64)>,
    inc_sum_div: (f64, f64),
    /// Field-variety repeat units (Σ excess over counted (team, field) pairs).
    /// Scored as hard when `field_variety_strict`, else soft × weight.
    team_field_count: HashMap<(usize, usize), u32>,
    inc_variety_units: f64,
    /// Specialist spread: per-(vol, div) counts + per-vol distinct-division count,
    /// summarised as Σ_vol max(0, distinct − 1).
    vol_div_count: Vec<Vec<u32>>,
    vol_distinct_divs: Vec<u32>,
    inc_specialist_units: f64,
    /// Per-slot occupancy for the peak-period variance (competition vs interview).
    comp_slot_occ: Vec<f64>,
    interview_slot_occ: Vec<f64>,
}

impl<'a> FastEvaluator<'a> {
    pub fn new(config: &'a InternalTournamentConfig, params: &'a SolverParams) -> Self {
        let num_teams = config.teams.len();
        let num_buckets = config.num_total_buckets;
        let num_fields = config.fields.len();
        let num_vols = config.volunteers.len();
        let num_days = config.days.len();
        let num_divs = config.divisions.len();

        Self {
            config,
            params,
            team_slot_occupancy: vec![vec![0; num_buckets]; num_teams],
            field_slot_occupancy: vec![vec![0; num_buckets]; num_fields],
            volunteer_slot_occupancy: vec![vec![0; num_buckets]; num_vols],
            volunteer_daily_counts: vec![vec![0; num_days]; num_vols],
            volunteer_shift_counts: vec![0; num_vols],
            field_match_counts: vec![0; num_fields],
            field_interview_counts: vec![0; num_fields],
            field_total_counts: vec![0; num_fields],
            team_day_assignments: vec![vec![Vec::new(); num_days]; num_teams],
            vol_day_assignments: vec![vec![Vec::new(); num_days]; num_vols],
            division_assignments: vec![Vec::new(); num_divs],
            current_hard_conflicts: 0.0,
            current_soft_penalties: 0.0,

            inc_ready: false,
            inc_local: Vec::new(),
            inc_sum_local: (0.0, 0.0),
            inc_occ_hard: 0.0,
            inc_team_day: vec![vec![(0.0, 0.0); num_days]; num_teams],
            inc_sum_team_day: (0.0, 0.0),
            inc_vol_day: vec![vec![0.0; num_days]; num_vols],
            inc_sum_vol_day: 0.0,
            inc_div: vec![(0.0, 0.0); num_divs],
            inc_sum_div: (0.0, 0.0),
            team_field_count: HashMap::new(),
            inc_variety_units: 0.0,
            vol_div_count: vec![vec![0; num_divs]; num_vols],
            vol_distinct_divs: vec![0; num_vols],
            inc_specialist_units: 0.0,
            comp_slot_occ: vec![0.0; config.slots.len()],
            interview_slot_occ: vec![0.0; config.slots.len()],
        }
    }

    pub fn init(&mut self, schedule: &InternalSchedule) {
        // Reset
        for row in &mut self.team_slot_occupancy { row.fill(0); }
        for row in &mut self.field_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_daily_counts { row.fill(0); }
        self.volunteer_shift_counts.fill(0);
        self.field_match_counts.fill(0);
        self.field_interview_counts.fill(0);
        self.field_total_counts.fill(0);
        for row in &mut self.team_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.vol_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.division_assignments { row.clear(); }

        // Populate state
        for (idx, assign) in schedule.assignments.iter().enumerate() {
            self.add_assignment_to_state(idx, assign);
        }

        // Initial cost (full scan once)
        self.calculate_total_cost(schedule);
    }

    fn add_assignment_to_state(&mut self, idx: usize, assign: &InternalAssignment) {
        let activity = &self.config.activities[idx];
        let day_idx = self.config.slots[assign.slot_idx].day_idx;

        for &t_idx in &activity.team_indices {
            self.team_day_assignments[t_idx][day_idx].push(idx);
        }
        for &v_idx in &assign.volunteer_indices {
            self.vol_day_assignments[v_idx][day_idx].push(idx);
        }
        self.division_assignments[activity.division_idx].push(idx);
    }

    /// Adds one assignment's contribution to the occupancy/count state used by
    /// the full-scan [`Self::evaluate`]: grouped lists, per-bucket occupancy,
    /// field load counts, and volunteer shift/daily counts. This is purely state
    /// population — no rules are evaluated here (see [`Self::report_assignment_local`]).
    fn add_assignment_counts(&mut self, idx: usize, assign: &InternalAssignment) {
        let activity = &self.config.activities[idx];
        let day_idx = self.config.slots[assign.slot_idx].day_idx;

        for &t_idx in &activity.team_indices {
            self.team_day_assignments[t_idx][day_idx].push(idx);
        }
        for &v_idx in &assign.volunteer_indices {
            self.vol_day_assignments[v_idx][day_idx].push(idx);
        }
        self.division_assignments[activity.division_idx].push(idx);

        let buckets = &self.config.activity_buckets[assign.slot_idx][activity.duration_class];
        for &b_idx in buckets {
            for &t_idx in &activity.team_indices {
                self.team_slot_occupancy[t_idx][b_idx] += 1;
            }
            if let Some(f_idx) = assign.field_idx {
                self.field_slot_occupancy[f_idx][b_idx] += 1;
            }
            for &v_idx in &assign.volunteer_indices {
                self.volunteer_slot_occupancy[v_idx][b_idx] += 1;
            }
        }

        if let Some(f_idx) = assign.field_idx {
            self.field_total_counts[f_idx] += 1;
            if activity.is_interview { self.field_interview_counts[f_idx] += 1; }
            else { self.field_match_counts[f_idx] += 1; }
        }

        for &v_idx in &assign.volunteer_indices {
            self.volunteer_shift_counts[v_idx] += 1;
            self.volunteer_daily_counts[v_idx][day_idx] += 1;
        }
    }

    /// Reports every hard/soft violation that depends only on a *single*
    /// assignment (slot-kind mismatch, field suitability, volunteer
    /// availability/qualification/conflict-of-interest, under-rostering,
    /// interviews-disabled, duration overflow, interview-late). It reads nothing
    /// from accumulated state, so it can be called either in the full scan or to
    /// recompute one assignment's local cost in the incremental path — keeping a
    /// single source of truth for these rules.
    fn report_assignment_local<S: ConflictSink>(
        params: &SolverParams,
        config: &InternalTournamentConfig,
        idx: usize,
        assign: &InternalAssignment,
        sink: &mut S,
    ) {
        let activity = &config.activities[idx];
        let multiplier = if activity.is_final { params.finals_priority_multiplier } else { 1.0 };
        let slot = &config.slots[assign.slot_idx];

        if (activity.is_interview && slot.kind == FieldKind::Competition) || (!activity.is_interview && slot.kind == FieldKind::Interview) {
            sink.report(CostClass::Hard, multiplier, ConflictKind::SlotKindMismatch, &[idx]);
        }

        if let Some(f_idx) = assign.field_idx {
            let f = &config.fields[f_idx];
            let mut suitable = true;
            if f.kind == FieldKind::Competition && activity.is_interview { suitable = false; }
            if f.kind == FieldKind::Interview && !activity.is_interview { suitable = false; }
            if let Some(ref allowed) = f.allowed_division_indices
                && !allowed.contains(&activity.division_idx) { suitable = false; }
            if !suitable {
                sink.report(CostClass::Hard, multiplier, ConflictKind::FieldUnsuitable { field_idx: f_idx }, &[idx]);
            }
        } else {
            sink.report(CostClass::Hard, multiplier, ConflictKind::FieldMissing, &[idx]);
        }

        let overlapped = &config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
        for &v_idx in &assign.volunteer_indices {
            let v = &config.volunteers[v_idx];

            for &s_idx in overlapped {
                if !v.availability_slots[s_idx] {
                    sink.report(CostClass::Hard, multiplier, ConflictKind::VolUnavailable { vol_idx: v_idx, slot_idx: s_idx }, &[idx]);
                }
            }

            let mut qualified = true;
            if activity.is_interview {
                if !config.can_interview[v_idx] {
                    if let Some(ref caps) = v.capability_indices {
                        if !caps.contains(&activity.division_idx) { qualified = false; }
                    } else {
                        qualified = true;
                    }
                } else {
                    qualified = true;
                }
            } else if let Some(ref caps) = v.capability_indices
            && !caps.contains(&activity.division_idx) { qualified = false; }

            if !qualified {
                if config.strict_capabilities || activity.is_interview {
                    sink.report(CostClass::Hard, multiplier, ConflictKind::VolUnqualified { vol_idx: v_idx }, &[idx]);
                } else {
                    sink.report(CostClass::Soft, params.vol_capability_weight * multiplier, ConflictKind::VolCapabilitySoft { vol_idx: v_idx }, &[idx]);
                }
            }

            for &t_idx in &activity.team_indices {
                let team = &config.teams[t_idx];
                if v.conflict_org_indices.contains(&team.org_idx) {
                    sink.report(CostClass::Hard, multiplier, ConflictKind::ConflictOfInterest { vol_idx: v_idx, team_idx: t_idx }, &[idx]);
                }
            }
        }

        let req = if activity.is_interview { config.divisions[activity.division_idx].interview_volunteers_required }
                  else { config.divisions[activity.division_idx].volunteers_required };
        if assign.volunteer_indices.len() < req {
            let missing = (req - assign.volunteer_indices.len()) as f64;
            sink.report(CostClass::Hard, missing * multiplier, ConflictKind::UnderRostered { required: req, assigned: assign.volunteer_indices.len() }, &[idx]);
        }

        if activity.is_interview && !config.day_interviews_enabled[slot.day_idx] {
            sink.report(CostClass::Hard, 10.0 * multiplier, ConflictKind::InterviewsDisabled, &[idx]);
        }
        let day_end = config.slots.iter().filter(|s| s.day_idx == slot.day_idx)
            .map(|s| s.start_minutes + s.duration_minutes).max().unwrap_or(0);
        if slot.start_minutes + activity.duration_minutes > day_end {
            sink.report(CostClass::Hard, multiplier, ConflictKind::DurationExceedsDay, &[idx]);
        }

        if activity.is_interview && params.interview_late_weight > 0.0 {
            sink.report(CostClass::Soft, (assign.slot_idx as f64) * params.interview_late_weight, ConflictKind::InterviewLate, &[idx]);
        }
    }

    /// Scores `schedule`, routing every hard and soft violation through `sink`.
    /// This is the single rule engine: [`Self::calculate_total_cost`],
    /// [`Self::collect_conflicts`], and [`Self::get_conflicted_indices`] are thin
    /// wrappers that differ only in which sink they pass, so the cost the solver
    /// minimises and the conflicts the UI shows are computed by the same code and
    /// cannot drift apart.
    pub fn evaluate<S: ConflictSink>(&mut self, schedule: &InternalSchedule, sink: &mut S) {
        let want_soft = sink.wants_soft();

        // Reset state
        for row in &mut self.team_slot_occupancy { row.fill(0); }
        for row in &mut self.field_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_daily_counts { row.fill(0); }
        self.volunteer_shift_counts.fill(0);
        self.field_match_counts.fill(0);
        self.field_interview_counts.fill(0);
        self.field_total_counts.fill(0);
        for row in &mut self.team_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.vol_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.division_assignments { row.clear(); }

        for (idx, assign) in schedule.assignments.iter().enumerate() {
            self.add_assignment_counts(idx, assign);
            Self::report_assignment_local(self.params, self.config, idx, assign, sink);
        }

        // Double booking + daily shift cap (hard), derived from the occupancy
        // state populated above.
        self.report_occupancy(schedule, sink);

        // Division stage ordering (hard) must always run so mutation targeting
        // can find it; round-order (soft) rides along in the same pass and is a
        // no-op for sinks that ignore soft.
        for div_idx in 0..self.config.divisions.len() {
            let list = &mut self.division_assignments[div_idx];
            list.sort_by_key(|&idx| (schedule.assignments[idx].slot_idx, idx));
            Self::report_division_penalties(self.params, self.config, schedule, list, sink);
        }

        // Field variety: strict repeats are hard (always evaluated); non-strict
        // repeats are a soft penalty.
        if self.params.field_variety_strict || self.config.divisions.iter().any(|d| d.mode == SchedulingMode::IndividualRun) {
            let mut team_field: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
            for (idx, assign) in schedule.assignments.iter().enumerate() {
                let Some(f_idx) = assign.field_idx else { continue };
                let activity = &self.config.activities[idx];
                let div_mode = self.config.divisions[activity.division_idx].mode;
                if !(self.params.field_variety_strict || div_mode == SchedulingMode::IndividualRun) {
                    continue;
                }
                for &t_idx in &activity.team_indices {
                    team_field.entry((t_idx, f_idx)).or_default().push(idx);
                }
            }
            for ((t_idx, f_idx), idxs) in &team_field {
                if idxs.len() > 1 {
                    let excess = (idxs.len() - 1) as f64;
                    if self.params.field_variety_strict {
                        sink.report(CostClass::Hard, excess, ConflictKind::FieldVarietyStrict { team_idx: *t_idx, field_idx: *f_idx }, idxs);
                    } else if want_soft {
                        sink.report(CostClass::Soft, excess * self.params.field_variety_weight, ConflictKind::FieldVariety, idxs);
                    }
                }
            }
        }

        // Team break rules. The hard minimum-break floor must be visible to
        // hard-only sinks (mutation targeting), so this runs above the soft
        // return; the soft buffer / wait-time / back-to-back parts inside the
        // helper are themselves gated on `want_soft`.
        for t_idx in 0..self.config.teams.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.team_day_assignments[t_idx][d_idx];
                list.sort_by_key(|&idx| (schedule.assignments[idx].slot_idx, idx));
                Self::report_team_day_penalties(self.params, self.config, schedule, list, t_idx, want_soft, sink);
            }
        }

        // Everything below is purely soft; sinks that only want hard conflicts
        // (mutation targeting) skip it entirely.
        if !want_soft {
            return;
        }

        for v_idx in 0..self.config.volunteers.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.vol_day_assignments[v_idx][d_idx];
                list.sort_by_key(|&idx| (schedule.assignments[idx].slot_idx, idx));
                Self::report_vol_day_penalties(self.params, self.config, schedule, list, sink);
            }
        }

        // Variance soft penalties
        sink.report(CostClass::Soft, calculate_variance(&self.field_match_counts) * self.params.field_balance_weight, ConflictKind::FieldBalance, &[]);
        sink.report(CostClass::Soft, calculate_variance(&self.field_interview_counts) * self.params.field_balance_weight, ConflictKind::FieldBalance, &[]);
        sink.report(CostClass::Soft, calculate_variance(&self.field_total_counts) * (self.params.field_balance_weight * 0.5), ConflictKind::FieldBalance, &[]);

        // Volunteer Fairness
        let fairness_mode = self.params.fairness_mode;
        let active_vols: Vec<f64> = self.volunteer_shift_counts.iter().enumerate().filter_map(|(v_idx, &count)| {
            let vol = &self.config.volunteers[v_idx];
            let avail_count = vol.availability_slots.iter().filter(|&&a| a).count() as f64;
            if avail_count == 0.0 { None } else { Some(count as f64 / avail_count) }
        }).collect();
        if !active_vols.is_empty() {
            let var = calculate_variance_f64(&active_vols);
            let weight = match fairness_mode { FairnessMode::Off => 5.0, FairnessMode::Balanced => 10.0, FairnessMode::Strict => 20.0 };
            sink.report(CostClass::Soft, var * weight, ConflictKind::VolFairness, &[]);
        }

        // Specialist Mode
        if self.params.vol_specialist_mode != SpecialistMode::Off {
            let weight = match self.params.vol_specialist_mode {
                SpecialistMode::Off => 0.0,
                SpecialistMode::Balanced => 0.5,
                SpecialistMode::Strict => 2.0,
            };
            let mut div_seen = vec![false; self.config.divisions.len()];
            for v_idx in 0..self.config.volunteers.len() {
                div_seen.fill(false);
                let mut count = 0;
                for d_idx in 0..self.config.days.len() {
                    for &idx in &self.vol_day_assignments[v_idx][d_idx] {
                        let div_idx = self.config.activities[idx].division_idx;
                        if !div_seen[div_idx] {
                            div_seen[div_idx] = true;
                            count += 1;
                        }
                    }
                }
                if count > 1 {
                    sink.report(CostClass::Soft, (count - 1) as f64 * weight, ConflictKind::Specialist, &[]);
                }
            }
        }

        // Peak period: encourage an even spread of activities across the whole
        // timeline by penalising the variance of per-time-slot occupancy. Competition
        // and interview slots are scored separately because they have very different
        // capacities (many fields vs a couple of interview tables) and would never
        // equalise if pooled. Crucially, *every* slot of each kind is included —
        // empty ones too — so a sparse early slot is penalised just like an
        // overloaded one; otherwise the solver is free to leave the start of a day
        // empty and pack the middle.
        if self.params.peak_period_weight > 0.0 {
            let mut comp_counts = vec![0.0f64; self.config.slots.len()];
            let mut interview_counts = vec![0.0f64; self.config.slots.len()];
            for (idx, assign) in schedule.assignments.iter().enumerate() {
                let activity = &self.config.activities[idx];
                let overlapped = &self.config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
                let counts = if activity.is_interview { &mut interview_counts } else { &mut comp_counts };
                for &s_idx in overlapped {
                    counts[s_idx] += 1.0;
                }
            }
            let by_kind = |kind: FieldKind, counts: &[f64]| -> Vec<f64> {
                self.config.slots.iter().enumerate()
                    .filter(|(_, s)| s.kind == kind)
                    .map(|(i, _)| counts[i])
                    .collect()
            };
            let comp = by_kind(FieldKind::Competition, &comp_counts);
            let interviews = by_kind(FieldKind::Interview, &interview_counts);
            let mut penalty = calculate_variance_f64(&comp);
            if !interviews.is_empty() {
                penalty += calculate_variance_f64(&interviews);
            }
            sink.report(CostClass::Soft, penalty * self.params.peak_period_weight, ConflictKind::PeakPeriod, &[]);
        }
    }

    /// Scalar `(hard, soft)` cost — the solver's hot path.
    pub fn calculate_total_cost(&mut self, schedule: &InternalSchedule) -> (f64, f64) {
        let mut sink = ScalarSink::default();
        self.evaluate(schedule, &mut sink);
        self.current_hard_conflicts = sink.hard;
        self.current_soft_penalties = sink.soft;
        (sink.hard, sink.soft)
    }

    /// Full structured conflict list for a schedule, used by the GUI to render
    /// diagnostics and counts.
    pub fn collect_conflicts(&mut self, schedule: &InternalSchedule) -> Vec<Conflict> {
        let mut sink = RecordSink::default();
        self.evaluate(schedule, &mut sink);
        sink.records
    }

    /// Emits double-booking and daily-shift-cap hard conflicts from the
    /// occupancy state populated by [`Self::evaluate`]. Each participating
    /// assignment is reported with a fractional weight that sums to the per-slot
    /// `(count - 1)` and per-day `(count - cap)` totals, so the scalar cost is
    /// exactly preserved while every involved assignment is still attributed
    /// (and thus targetable by mutation / showable in the UI).
    fn report_occupancy<S: ConflictSink>(&self, schedule: &InternalSchedule, sink: &mut S) {
        let cap = self.params.vol_daily_shift_cap;
        for (idx, assign) in schedule.assignments.iter().enumerate() {
            let activity = &self.config.activities[idx];
            let day_idx = self.config.slots[assign.slot_idx].day_idx;
            let buckets = &self.config.activity_buckets[assign.slot_idx][activity.duration_class];

            for &t_idx in &activity.team_indices {
                for &b in buckets {
                    let c = self.team_slot_occupancy[t_idx][b];
                    if c > 1 {
                        sink.report(CostClass::Hard, (c - 1) as f64 / c as f64, ConflictKind::TeamDoubleBooked { team_idx: t_idx }, &[idx]);
                    }
                }
            }
            if let Some(f_idx) = assign.field_idx {
                for &b in buckets {
                    let c = self.field_slot_occupancy[f_idx][b];
                    if c > 1 {
                        sink.report(CostClass::Hard, (c - 1) as f64 / c as f64, ConflictKind::FieldDoubleBooked { field_idx: f_idx }, &[idx]);
                    }
                }
            }
            for &v_idx in &assign.volunteer_indices {
                for &b in buckets {
                    let c = self.volunteer_slot_occupancy[v_idx][b];
                    if c > 1 {
                        sink.report(CostClass::Hard, (c - 1) as f64 / c as f64, ConflictKind::VolDoubleBooked { vol_idx: v_idx }, &[idx]);
                    }
                }
                if cap > 0 {
                    let dc = self.volunteer_daily_counts[v_idx][day_idx];
                    if dc > cap as u32 {
                        sink.report(CostClass::Hard, (dc - cap as u32) as f64 / dc as f64, ConflictKind::DailyShiftCapExceeded { vol_idx: v_idx }, &[idx]);
                    }
                }
            }
        }
    }

    /// Indices of assignments involved in at least one hard conflict, used to
    /// bias mutation toward broken assignments. Derived from the same engine as
    /// the cost, so it now covers every hard rule — including division stage
    /// ordering, which the previous hand-written scan silently missed.
    pub fn get_conflicted_indices(&mut self, schedule: &InternalSchedule) -> Vec<usize> {
        // Once the incrementally-tracked schedule is feasible (zero hard cost),
        // no assignment is in a hard conflict, so the set is empty without a
        // scan. This is the common case during the long soft-optimisation phase,
        // so it keeps the per-iteration cost O(1) there.
        if self.inc_ready && self.inc_hard() < 0.5 {
            return Vec::new();
        }
        let mut sink = ConflictedSink::new(schedule.assignments.len());
        self.evaluate(schedule, &mut sink);
        sink.into_indices()
    }

    // ===================== Incremental (delta) evaluation =====================
    //
    // The solver mutates one or two assignments per iteration and almost always
    // reverts. Re-scanning the whole schedule each time is the dominant cost, so
    // these methods maintain every cost component as a running total: a mutation
    // detaches the old assignment(s), attaches the new, and recomputes only the
    // partitions (team-day, vol-day, division, local) it actually touched. The
    // small variance aggregates are recomputed from live counts in `inc_total`.
    // Correctness against the full scan is asserted in
    // `solver::tests::incremental_matches_full_recompute`.

    /// True if the field-variety rule is active at all (strict, or some division
    /// runs in individual mode).
    fn variety_active(&self) -> bool {
        self.params.field_variety_strict
            || self.config.divisions.iter().any(|d| d.mode == SchedulingMode::IndividualRun)
    }

    /// Whether assignment `idx`'s activity contributes to field-variety counting.
    fn counts_for_variety(&self, div_idx: usize) -> bool {
        self.params.field_variety_strict
            || self.config.divisions[div_idx].mode == SchedulingMode::IndividualRun
    }

    /// Adds one assignment's contribution to all live state and the incremental
    /// running scalars (occupancy, variety, specialist, peak). Inverse of
    /// [`Self::detach_one`].
    fn attach_one(&mut self, idx: usize, assign: &InternalAssignment) {
        let config = self.config;
        let activity = &config.activities[idx];
        let day_idx = config.slots[assign.slot_idx].day_idx;

        for &t in &activity.team_indices { self.team_day_assignments[t][day_idx].push(idx); }
        for &v in &assign.volunteer_indices { self.vol_day_assignments[v][day_idx].push(idx); }
        self.division_assignments[activity.division_idx].push(idx);

        let buckets = &config.activity_buckets[assign.slot_idx][activity.duration_class];
        for &b in buckets {
            for &t in &activity.team_indices {
                let c = self.team_slot_occupancy[t][b];
                if c >= 1 { self.inc_occ_hard += 1.0; }
                self.team_slot_occupancy[t][b] = c + 1;
            }
            if let Some(f) = assign.field_idx {
                let c = self.field_slot_occupancy[f][b];
                if c >= 1 { self.inc_occ_hard += 1.0; }
                self.field_slot_occupancy[f][b] = c + 1;
            }
            for &v in &assign.volunteer_indices {
                let c = self.volunteer_slot_occupancy[v][b];
                if c >= 1 { self.inc_occ_hard += 1.0; }
                self.volunteer_slot_occupancy[v][b] = c + 1;
            }
        }

        if let Some(f) = assign.field_idx {
            self.field_total_counts[f] += 1;
            if activity.is_interview { self.field_interview_counts[f] += 1; } else { self.field_match_counts[f] += 1; }
        }

        let cap = self.params.vol_daily_shift_cap;
        for &v in &assign.volunteer_indices {
            self.volunteer_shift_counts[v] += 1;
            let dc = self.volunteer_daily_counts[v][day_idx];
            if cap > 0 && dc >= cap as u32 { self.inc_occ_hard += 1.0; }
            self.volunteer_daily_counts[v][day_idx] = dc + 1;
        }

        if self.variety_active() && self.counts_for_variety(activity.division_idx) {
            if let Some(f) = assign.field_idx {
                for &t in &activity.team_indices {
                    let key = (t, f);
                    let c = *self.team_field_count.get(&key).unwrap_or(&0);
                    if c >= 1 { self.inc_variety_units += 1.0; }
                    self.team_field_count.insert(key, c + 1);
                }
            }
        }

        if self.params.vol_specialist_mode != SpecialistMode::Off {
            let div = activity.division_idx;
            for &v in &assign.volunteer_indices {
                let c = self.vol_div_count[v][div];
                self.vol_div_count[v][div] = c + 1;
                if c == 0 {
                    self.vol_distinct_divs[v] += 1;
                    if self.vol_distinct_divs[v] >= 2 { self.inc_specialist_units += 1.0; }
                }
            }
        }

        let overlapped = &config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
        if activity.is_interview {
            for &s in overlapped { self.interview_slot_occ[s] += 1.0; }
        } else {
            for &s in overlapped { self.comp_slot_occ[s] += 1.0; }
        }
    }

    /// Removes one assignment's contribution from all live state and the
    /// incremental running scalars. Exact inverse of [`Self::attach_one`].
    fn detach_one(&mut self, idx: usize, assign: &InternalAssignment) {
        let config = self.config;
        let activity = &config.activities[idx];
        let day_idx = config.slots[assign.slot_idx].day_idx;

        for &t in &activity.team_indices { remove_val(&mut self.team_day_assignments[t][day_idx], idx); }
        for &v in &assign.volunteer_indices { remove_val(&mut self.vol_day_assignments[v][day_idx], idx); }
        remove_val(&mut self.division_assignments[activity.division_idx], idx);

        let buckets = &config.activity_buckets[assign.slot_idx][activity.duration_class];
        for &b in buckets {
            for &t in &activity.team_indices {
                let c = self.team_slot_occupancy[t][b];
                if c >= 2 { self.inc_occ_hard -= 1.0; }
                self.team_slot_occupancy[t][b] = c - 1;
            }
            if let Some(f) = assign.field_idx {
                let c = self.field_slot_occupancy[f][b];
                if c >= 2 { self.inc_occ_hard -= 1.0; }
                self.field_slot_occupancy[f][b] = c - 1;
            }
            for &v in &assign.volunteer_indices {
                let c = self.volunteer_slot_occupancy[v][b];
                if c >= 2 { self.inc_occ_hard -= 1.0; }
                self.volunteer_slot_occupancy[v][b] = c - 1;
            }
        }

        if let Some(f) = assign.field_idx {
            self.field_total_counts[f] -= 1;
            if activity.is_interview { self.field_interview_counts[f] -= 1; } else { self.field_match_counts[f] -= 1; }
        }

        let cap = self.params.vol_daily_shift_cap;
        for &v in &assign.volunteer_indices {
            self.volunteer_shift_counts[v] -= 1;
            let dc = self.volunteer_daily_counts[v][day_idx];
            if cap > 0 && dc > cap as u32 { self.inc_occ_hard -= 1.0; }
            self.volunteer_daily_counts[v][day_idx] = dc - 1;
        }

        if self.variety_active() && self.counts_for_variety(activity.division_idx) {
            if let Some(f) = assign.field_idx {
                for &t in &activity.team_indices {
                    let key = (t, f);
                    let c = *self.team_field_count.get(&key).unwrap_or(&0);
                    if c >= 2 { self.inc_variety_units -= 1.0; }
                    if c <= 1 { self.team_field_count.remove(&key); } else { self.team_field_count.insert(key, c - 1); }
                }
            }
        }

        if self.params.vol_specialist_mode != SpecialistMode::Off {
            let div = activity.division_idx;
            for &v in &assign.volunteer_indices {
                let c = self.vol_div_count[v][div];
                self.vol_div_count[v][div] = c - 1;
                if c == 1 {
                    if self.vol_distinct_divs[v] >= 2 { self.inc_specialist_units -= 1.0; }
                    self.vol_distinct_divs[v] -= 1;
                }
            }
        }

        let overlapped = &config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
        if activity.is_interview {
            for &s in overlapped { self.interview_slot_occ[s] -= 1.0; }
        } else {
            for &s in overlapped { self.comp_slot_occ[s] -= 1.0; }
        }
    }

    fn recompute_local(&mut self, idx: usize, assign: &InternalAssignment) {
        let mut sink = ScalarSink::default();
        Self::report_assignment_local(self.params, self.config, idx, assign, &mut sink);
        let (oh, os) = self.inc_local[idx];
        self.inc_sum_local.0 += sink.hard - oh;
        self.inc_sum_local.1 += sink.soft - os;
        self.inc_local[idx] = (sink.hard, sink.soft);
    }

    fn recompute_team_day(&mut self, schedule: &InternalSchedule, t: usize, d: usize) {
        self.team_day_assignments[t][d].sort_by_key(|&i| (schedule.assignments[i].slot_idx, i));
        let mut sink = ScalarSink::default();
        Self::report_team_day_penalties(self.params, self.config, schedule, &self.team_day_assignments[t][d], t, true, &mut sink);
        let (oh, os) = self.inc_team_day[t][d];
        self.inc_sum_team_day.0 += sink.hard - oh;
        self.inc_sum_team_day.1 += sink.soft - os;
        self.inc_team_day[t][d] = (sink.hard, sink.soft);
    }

    fn recompute_vol_day(&mut self, schedule: &InternalSchedule, v: usize, d: usize) {
        self.vol_day_assignments[v][d].sort_by_key(|&i| (schedule.assignments[i].slot_idx, i));
        let mut sink = ScalarSink::default();
        Self::report_vol_day_penalties(self.params, self.config, schedule, &self.vol_day_assignments[v][d], &mut sink);
        let old = self.inc_vol_day[v][d];
        self.inc_sum_vol_day += sink.soft - old;
        self.inc_vol_day[v][d] = sink.soft;
    }

    fn recompute_div(&mut self, schedule: &InternalSchedule, div: usize) {
        self.division_assignments[div].sort_by_key(|&i| (schedule.assignments[i].slot_idx, i));
        let mut sink = ScalarSink::default();
        Self::report_division_penalties(self.params, self.config, schedule, &self.division_assignments[div], &mut sink);
        let (oh, os) = self.inc_div[div];
        self.inc_sum_div.0 += sink.hard - oh;
        self.inc_sum_div.1 += sink.soft - os;
        self.inc_div[div] = (sink.hard, sink.soft);
    }

    /// Builds the full incremental state for `schedule` from scratch (O(N)).
    /// Call once per restart, then maintain it with [`Self::apply_change`].
    pub fn inc_init(&mut self, schedule: &InternalSchedule) {
        for row in &mut self.team_slot_occupancy { row.fill(0); }
        for row in &mut self.field_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_slot_occupancy { row.fill(0); }
        for row in &mut self.volunteer_daily_counts { row.fill(0); }
        self.volunteer_shift_counts.fill(0);
        self.field_match_counts.fill(0);
        self.field_interview_counts.fill(0);
        self.field_total_counts.fill(0);
        for row in &mut self.team_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.vol_day_assignments { for col in row { col.clear(); } }
        for row in &mut self.division_assignments { row.clear(); }

        self.inc_local = vec![(0.0, 0.0); schedule.assignments.len()];
        self.inc_sum_local = (0.0, 0.0);
        self.inc_occ_hard = 0.0;
        for row in &mut self.inc_team_day { for c in row { *c = (0.0, 0.0); } }
        self.inc_sum_team_day = (0.0, 0.0);
        for row in &mut self.inc_vol_day { for c in row { *c = 0.0; } }
        self.inc_sum_vol_day = 0.0;
        for c in &mut self.inc_div { *c = (0.0, 0.0); }
        self.inc_sum_div = (0.0, 0.0);
        self.team_field_count.clear();
        self.inc_variety_units = 0.0;
        for row in &mut self.vol_div_count { row.fill(0); }
        self.vol_distinct_divs.fill(0);
        self.inc_specialist_units = 0.0;
        self.comp_slot_occ.fill(0.0);
        self.interview_slot_occ.fill(0.0);

        for (idx, assign) in schedule.assignments.iter().enumerate() {
            self.attach_one(idx, assign);
        }
        for idx in 0..schedule.assignments.len() {
            self.recompute_local(idx, &schedule.assignments[idx]);
        }
        for t in 0..self.config.teams.len() {
            for d in 0..self.config.days.len() { self.recompute_team_day(schedule, t, d); }
        }
        for v in 0..self.config.volunteers.len() {
            for d in 0..self.config.days.len() { self.recompute_vol_day(schedule, v, d); }
        }
        for div in 0..self.config.divisions.len() { self.recompute_div(schedule, div); }
        self.inc_ready = true;
    }

    /// Current incremental hard cost (O(1)). Used to short-circuit
    /// [`Self::get_conflicted_indices`] once the schedule is feasible.
    fn inc_hard(&self) -> f64 {
        self.inc_sum_local.0 + self.inc_occ_hard + self.inc_sum_team_day.0 + self.inc_sum_div.0
            + if self.params.field_variety_strict { self.inc_variety_units } else { 0.0 }
    }

    /// The maintained `(hard, soft)` cost: O(1) running totals for the
    /// partitioned terms plus a cheap recompute of the small variance aggregates
    /// (field balance, fairness, specialist, peak) from live counts.
    pub fn inc_total(&self) -> (f64, f64) {
        let hard = self.inc_hard();
        let mut soft = self.inc_sum_local.1 + self.inc_sum_team_day.1 + self.inc_sum_vol_day + self.inc_sum_div.1;
        if !self.params.field_variety_strict {
            soft += self.inc_variety_units * self.params.field_variety_weight;
        }

        let w = self.params.field_balance_weight;
        soft += calculate_variance(&self.field_match_counts) * w;
        soft += calculate_variance(&self.field_interview_counts) * w;
        soft += calculate_variance(&self.field_total_counts) * (w * 0.5);

        let active_vols: Vec<f64> = self.volunteer_shift_counts.iter().enumerate().filter_map(|(v, &count)| {
            let vol = &self.config.volunteers[v];
            let avail = vol.availability_slots.iter().filter(|&&a| a).count() as f64;
            if avail == 0.0 { None } else { Some(count as f64 / avail) }
        }).collect();
        if !active_vols.is_empty() {
            let var = calculate_variance_f64(&active_vols);
            let weight = match self.params.fairness_mode { FairnessMode::Off => 5.0, FairnessMode::Balanced => 10.0, FairnessMode::Strict => 20.0 };
            soft += var * weight;
        }

        if self.params.vol_specialist_mode != SpecialistMode::Off {
            let weight = match self.params.vol_specialist_mode {
                SpecialistMode::Off => 0.0, SpecialistMode::Balanced => 0.5, SpecialistMode::Strict => 2.0,
            };
            soft += self.inc_specialist_units * weight;
        }

        if self.params.peak_period_weight > 0.0 {
            let by_kind = |kind: FieldKind, counts: &[f64]| -> Vec<f64> {
                self.config.slots.iter().enumerate().filter(|(_, s)| s.kind == kind).map(|(i, _)| counts[i]).collect()
            };
            let comp = by_kind(FieldKind::Competition, &self.comp_slot_occ);
            let interviews = by_kind(FieldKind::Interview, &self.interview_slot_occ);
            let mut penalty = calculate_variance_f64(&comp);
            if !interviews.is_empty() { penalty += calculate_variance_f64(&interviews); }
            soft += penalty * self.params.peak_period_weight;
        }

        (hard, soft)
    }

    /// Applies a set of assignment changes incrementally and returns the new
    /// `(hard, soft)` cost. `changes` holds `(idx, prior_assignment)` for each
    /// touched assignment; `schedule` already reflects the *new* values. To
    /// revert, call again with the same indices but the new values as `prior`
    /// and `schedule` restored — the operation is its own inverse.
    pub fn apply_change(&mut self, schedule: &InternalSchedule, changes: &[(usize, InternalAssignment)]) -> (f64, f64) {
        for (idx, prior) in changes { self.detach_one(*idx, prior); }
        for (idx, _) in changes { self.attach_one(*idx, &schedule.assignments[*idx]); }

        let config = self.config;
        for (idx, prior) in changes {
            let new = &schedule.assignments[*idx];
            self.recompute_local(*idx, new);

            let activity = &config.activities[*idx];
            let old_day = config.slots[prior.slot_idx].day_idx;
            let new_day = config.slots[new.slot_idx].day_idx;
            let days: &[usize] = if old_day == new_day { &[old_day][..] } else { &[old_day, new_day][..] };

            for &t in &activity.team_indices {
                for &d in days { self.recompute_team_day(schedule, t, d); }
            }
            let mut vols = prior.volunteer_indices.clone();
            for &v in &new.volunteer_indices { if !vols.contains(&v) { vols.push(v); } }
            for &v in &vols {
                for &d in days { self.recompute_vol_day(schedule, v, d); }
            }
            self.recompute_div(schedule, activity.division_idx);
        }
        self.inc_total()
    }

    fn report_team_day_penalties<S: ConflictSink>(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize], team_idx: usize, want_soft: bool, sink: &mut S) {
        if list.is_empty() { return; }

        // Break enforcement between a team's consecutive activities. The gap is
        // the real wall-clock time (minutes) between the end of one activity and
        // the start of the next — `list` is sorted chronologically, so adjacent
        // entries are the pair whose gap actually binds. Interview↔match uses the
        // interview-break settings; match↔match uses the recharge-break settings
        // (a global floor with an optional per-division override).
        for w in list.windows(2) {
            let (i1, i2) = (w[0], w[1]);
            let act1 = &config.activities[i1];
            let act2 = &config.activities[i2];
            let slot1 = &config.slots[schedule.assignments[i1].slot_idx];
            let slot2 = &config.slots[schedule.assignments[i2].slot_idx];
            if slot1.day_idx != slot2.day_idx { continue; }

            let end1 = slot1.start_minutes + act1.duration_minutes;
            // Overlaps are already a hard double-booking; skip them here.
            if slot2.start_minutes < end1 { continue; }
            let gap = slot2.start_minutes - end1;
            let mult = if act1.is_final || act2.is_final { params.finals_priority_multiplier } else { 1.0 };

            if act1.is_interview != act2.is_interview {
                // Interview ↔ match.
                if params.team_min_break_minutes > 0 && gap < params.team_min_break_minutes {
                    sink.report(CostClass::Hard, mult, ConflictKind::TeamMinBreak { team_idx }, &[i1, i2]);
                }
                if want_soft
                    && params.interview_match_gap_weight > 0.0
                    && params.team_break_buffer_minutes > 0
                    && gap < params.team_break_buffer_minutes {
                    let shortfall = (params.team_break_buffer_minutes - gap) as f64 / params.team_break_buffer_minutes as f64;
                    sink.report(CostClass::Soft, shortfall * params.interview_match_gap_weight, ConflictKind::InterviewMatchGap, &[i1, i2]);
                }
            } else if !act1.is_interview {
                // Match ↔ match (robot recharge). A per-division override, when set,
                // replaces the global floor for this team's division.
                let floor = config.divisions[act1.division_idx]
                    .min_match_break_minutes
                    .unwrap_or(params.team_match_min_break_minutes);
                if floor > 0 && gap < floor {
                    sink.report(CostClass::Hard, mult, ConflictKind::TeamMatchBreak { team_idx }, &[i1, i2]);
                }
                if want_soft
                    && params.team_back_to_back_weight > 0.0
                    && params.team_match_break_buffer_minutes > 0
                    && gap < params.team_match_break_buffer_minutes {
                    let shortfall = (params.team_match_break_buffer_minutes - gap) as f64 / params.team_match_break_buffer_minutes as f64;
                    sink.report(CostClass::Soft, shortfall * params.team_back_to_back_weight, ConflictKind::TeamBackToBack, &[i1, i2]);
                }
            }
            // Interview ↔ interview: no break rule.
        }

        if !want_soft { return; }

        // Wait Time
        if params.team_wait_time_weight > 0.0 && list.len() > 1 {
            let non_interviews: Vec<usize> = list.iter().filter(|&&i| !config.activities[i].is_interview).copied().collect();
            if non_interviews.len() > 1 {
                let first = non_interviews[0];
                let last = *non_interviews.last().unwrap();
                let span = schedule.assignments[last].slot_idx - schedule.assignments[first].slot_idx;
                let excessive_span = span.saturating_sub(non_interviews.len() * 2);
                if excessive_span > 0 {
                    sink.report(CostClass::Soft, (excessive_span as f64) * params.team_wait_time_weight, ConflictKind::TeamWaitTime, &[first, last]);
                }
            }
        }

    }

    fn report_vol_day_penalties<S: ConflictSink>(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize], sink: &mut S) {
        if list.len() < 2 { return; }
        for window in list.windows(2) {
            let i1 = window[0];
            let i2 = window[1];
            let assign1 = &schedule.assignments[i1];
            let assign2 = &schedule.assignments[i2];
            let act1 = &config.activities[i1];

            let slot1 = &config.slots[assign1.slot_idx];
            let slot2 = &config.slots[assign2.slot_idx];
            if slot1.day_idx == slot2.day_idx
                && slot2.start_minutes == slot1.start_minutes + act1.duration_minutes {
                sink.report(CostClass::Soft, params.vol_consecutive_weight, ConflictKind::VolConsecutive, &[i1, i2]);
                if params.vol_travel_weight > 0.0 && assign1.field_idx != assign2.field_idx {
                    sink.report(CostClass::Soft, params.vol_travel_weight, ConflictKind::VolTravel, &[i1, i2]);
                }
            }
        }
    }

    fn report_division_penalties<S: ConflictSink>(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize], sink: &mut S) {
        if list.is_empty() { return; }
        if params.round_order_weight < 0.0 { return; }

        for i in 0..list.len() {
            let idx1 = list[i];
            let act1 = &config.activities[idx1];
            if act1.is_interview { continue; }
            let round1 = act1.round_index;
            let stage1 = act1.stage;
            let assign1 = &schedule.assignments[idx1];
            let slot1 = &config.slots[assign1.slot_idx];
            let end1 = slot1.start_minutes + act1.duration_minutes;

            for &idx2 in list.iter().skip(i + 1) {
                let act2 = &config.activities[idx2];
                if act2.is_interview { continue; }
                let round2 = act2.round_index;
                let stage2 = act2.stage;
                let assign2 = &schedule.assignments[idx2];
                let slot2 = &config.slots[assign2.slot_idx];

                // list is sorted by slot_idx, so slot1.idx <= slot2.idx is guaranteed for i < j.

                if stage1 != stage2 {
                    let is_3pl_gf = (stage1 == 4 && stage2 == 5) || (stage1 == 5 && stage2 == 4);
                    if !is_3pl_gf {
                        if stage1 > stage2 {
                            // Hard conflict: later stage starts at or before earlier stage
                            sink.report(CostClass::Hard, 10.0, ConflictKind::StageOrder, &[idx1, idx2]);
                        } else if slot1.day_idx == slot2.day_idx && end1 > slot2.start_minutes {
                            // stage1 < stage2: overlap between different stages.
                            sink.report(CostClass::Hard, 10.0, ConflictKind::StageOverlap, &[idx1, idx2]);
                        }
                    }
                } else if params.round_order_weight > 0.0 {
                    // Same stage: enforce round order with soft penalty

                    // Same day check
                    if slot1.day_idx != slot2.day_idx {
                        if round1 > round2 {
                            sink.report(CostClass::Soft, params.round_order_weight, ConflictKind::RoundOrder, &[idx1, idx2]);
                        }
                        continue;
                    }

                    if round1 > round2 {
                        // Strictly out of order
                        sink.report(CostClass::Soft, params.round_order_weight, ConflictKind::RoundOrder, &[idx1, idx2]);
                    } else if round1 < round2
                        && end1 > slot2.start_minutes && assign1.slot_idx == assign2.slot_idx {
                            // Traditional same-start overlap penalty for same-stage activities
                            sink.report(CostClass::Soft, params.round_order_weight * 0.5, ConflictKind::RoundOrder, &[idx1, idx2]);
                        }
                }
            }
        }
    }
}

/// Removes the first occurrence of `val` from `v` (order not preserved). The
/// grouped lists are re-sorted before use, so `swap_remove` is safe and O(1).
fn remove_val(v: &mut Vec<usize>, val: usize) {
    if let Some(pos) = v.iter().position(|&x| x == val) {
        v.swap_remove(pos);
    }
}

fn calculate_variance(counts: &[u32]) -> f64 {
    if counts.is_empty() { return 0.0; }
    let sum: u32 = counts.iter().sum();
    let mean = sum as f64 / counts.len() as f64;
    counts.iter().map(|&c| (c as f64 - mean).powi(2)).sum::<f64>() / counts.len() as f64
}

fn calculate_variance_f64(values: &[f64]) -> f64 {
    if values.is_empty() { return 0.0; }
    let sum: f64 = values.iter().sum();
    let mean = sum / values.len() as f64;
    values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

/// Evaluates the cost of a (model-level) `Schedule` using the same engine the
/// solver optimizes against. This is the single source of truth for schedule
/// cost: the GUI and the solver both score schedules through this path, so the
/// number shown to the user always matches what was optimized.
pub fn evaluate_schedule_cost(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> (f64, f64) {
    // Activities not belonging to a known division can't be compiled; count each
    // as a hard conflict (matching the previous evaluator's behavior) and drop it.
    let valid: Vec<&crate::model::ScheduleAssignment> = schedule
        .assignments
        .iter()
        .filter(|a| config.divisions.iter().any(|d| d.id == a.activity.division_id()))
        .collect();
    let dropped = (schedule.assignments.len() - valid.len()) as f64;

    if valid.is_empty() {
        return (dropped, 0.0);
    }

    let activities: Vec<crate::model::Activity> = valid.iter().map(|a| a.activity.clone()).collect();
    let internal_config = InternalTournamentConfig::compile(config, &activities);

    let slot_idx: HashMap<&str, usize> =
        internal_config.slots.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    let field_idx: HashMap<&str, usize> =
        internal_config.fields.iter().enumerate().map(|(i, f)| (f.id.as_str(), i)).collect();
    let vol_idx: HashMap<&str, usize> =
        internal_config.volunteers.iter().enumerate().map(|(i, v)| (v.id.as_str(), i)).collect();

    let assignments: Vec<InternalAssignment> = valid
        .iter()
        .map(|a| InternalAssignment {
            slot_idx: slot_idx.get(a.time_slot_id.as_str()).copied().unwrap_or(0),
            field_idx: a.field_id.as_ref().and_then(|f| field_idx.get(f.as_str()).copied()),
            volunteer_indices: a
                .volunteer_ids
                .iter()
                .filter_map(|v| vol_idx.get(v.as_str()).copied())
                .collect(),
        })
        .collect();

    let internal_schedule = InternalSchedule { assignments };
    let mut evaluator = FastEvaluator::new(&internal_config, params);
    evaluator.init(&internal_schedule);
    let (hard, soft) = evaluator.calculate_total_cost(&internal_schedule);
    (hard + dropped, soft)
}

/// Compiles a model `Schedule`, scores it through the unified engine, and
/// returns the structured conflicts. The `who` indices on each returned
/// [`Conflict`] are remapped back to positions in `schedule.assignments`.
///
/// Returns the compiled `InternalTournamentConfig` (so callers can resolve the
/// internal field/volunteer/team indices carried in each `ConflictKind` to
/// names) and the model indices of any assignments whose division is unknown —
/// these can't be compiled and are surfaced separately as hard problems.
pub fn evaluate_schedule_conflicts(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> (InternalTournamentConfig, Vec<Conflict>, Vec<usize>) {
    let mut model_index = Vec::new();
    let mut dropped = Vec::new();
    let valid: Vec<&ScheduleAssignment> = schedule
        .assignments
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            if config.divisions.iter().any(|d| d.id == a.activity.division_id()) {
                model_index.push(i);
                Some(a)
            } else {
                dropped.push(i);
                None
            }
        })
        .collect();

    if valid.is_empty() {
        let internal_config = InternalTournamentConfig::compile(config, &[]);
        return (internal_config, Vec::new(), dropped);
    }

    let activities: Vec<Activity> = valid.iter().map(|a| a.activity.clone()).collect();
    let internal_config = InternalTournamentConfig::compile(config, &activities);

    let slot_idx: HashMap<&str, usize> =
        internal_config.slots.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    let field_idx: HashMap<&str, usize> =
        internal_config.fields.iter().enumerate().map(|(i, f)| (f.id.as_str(), i)).collect();
    let vol_idx: HashMap<&str, usize> =
        internal_config.volunteers.iter().enumerate().map(|(i, v)| (v.id.as_str(), i)).collect();

    let assignments: Vec<InternalAssignment> = valid
        .iter()
        .map(|a| InternalAssignment {
            slot_idx: slot_idx.get(a.time_slot_id.as_str()).copied().unwrap_or(0),
            field_idx: a.field_id.as_ref().and_then(|f| field_idx.get(f.as_str()).copied()),
            volunteer_indices: a
                .volunteer_ids
                .iter()
                .filter_map(|v| vol_idx.get(v.as_str()).copied())
                .collect(),
        })
        .collect();

    let internal_schedule = InternalSchedule { assignments };
    let mut evaluator = FastEvaluator::new(&internal_config, params);
    let mut records = evaluator.collect_conflicts(&internal_schedule);

    // Remap who from internal (valid-only) positions to original model positions.
    for c in &mut records {
        for w in &mut c.who {
            *w = model_index[*w];
        }
    }

    (internal_config, records, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::scheduler::internal::InternalTournamentConfig;
    use crate::scheduler::SolverParams;
    use crate::model::Activity;

    #[test]
    fn test_fast_evaluator_sf_3pl_order() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 2, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: true, finals_rounds: Some(FinalsRounds::Semis), finals_duration_minutes: Some(20),
                finals_third_place_playoff: true,
                color: None, min_match_break_minutes: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![
            DayGenConfig { day: "Sat".into(), ..Default::default() },
        ];

        let activities = vec![
            Activity::Match { id: "div1_3pl".into(), team_a: "L1".into(), team_b: "L2".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::ThirdPlace }, 
            Activity::Match { id: "div1_sf_1".into(), team_a: "1st".into(), team_b: "4th".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::SemiFinal }, 
        ];

        let internal_config = InternalTournamentConfig::compile(&config, &activities);
        let params = SolverParams::default();
        let mut evaluator = FastEvaluator::new(&internal_config, &params);

        let schedule = crate::scheduler::internal::InternalSchedule {
            assignments: vec![
                crate::scheduler::internal::InternalAssignment { slot_idx: 0, field_idx: Some(0), volunteer_indices: vec![] },
                crate::scheduler::internal::InternalAssignment { slot_idx: 1, field_idx: Some(1), volunteer_indices: vec![] },
            ]
        };

        evaluator.init(&schedule);
        let cost = evaluator.calculate_total_cost(&schedule);

        assert!(cost.0 >= 10.0, "3PL before SF should be a hard conflict in FastEvaluator (got {})", cost.0);
    }

    /// Builds a tiny config + schedule with a guaranteed stage-order hard
    /// conflict (3rd-place playoff scheduled before the semi-final).
    fn sf3pl() -> (InternalTournamentConfig, crate::scheduler::internal::InternalSchedule) {
        let mut config = TournamentConfig::default();
        config.divisions = vec![Division {
            id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 2, volunteers_required: 0, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: false,
            interview_volunteers_required: 0, interview_duration_minutes: 0,
            finals_enabled: true, finals_rounds: Some(FinalsRounds::Semis), finals_duration_minutes: Some(20),
            finals_third_place_playoff: true, color: None, min_match_break_minutes: None,
        }];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![DayGenConfig { day: "Sat".into(), ..Default::default() }];

        let activities = vec![
            Activity::Match { id: "div1_3pl".into(), team_a: "L1".into(), team_b: "L2".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::ThirdPlace },
            Activity::Match { id: "div1_sf_1".into(), team_a: "1st".into(), team_b: "4th".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::SemiFinal },
        ];
        let internal_config = InternalTournamentConfig::compile(&config, &activities);
        let schedule = crate::scheduler::internal::InternalSchedule {
            assignments: vec![
                crate::scheduler::internal::InternalAssignment { slot_idx: 0, field_idx: Some(0), volunteer_indices: vec![] },
                crate::scheduler::internal::InternalAssignment { slot_idx: 1, field_idx: Some(1), volunteer_indices: vec![] },
            ],
        };
        (internal_config, schedule)
    }

    /// The core unification invariant: the scalar cost the solver minimises is
    /// exactly the sum of the structured record weights the GUI displays.
    #[test]
    fn scalar_and_record_sinks_agree() {
        use crate::scheduler::conflicts::{CostClass, RecordSink, ScalarSink};
        let (cfg, sched) = sf3pl();
        let params = SolverParams::default();
        let mut e = FastEvaluator::new(&cfg, &params);

        let mut scalar = ScalarSink::default();
        e.evaluate(&sched, &mut scalar);
        let mut rec = RecordSink::default();
        e.evaluate(&sched, &mut rec);

        let hard_sum: f64 = rec.records.iter().filter(|c| c.class == CostClass::Hard).map(|c| c.weight).sum();
        let soft_sum: f64 = rec.records.iter().filter(|c| c.class == CostClass::Soft).map(|c| c.weight).sum();
        assert!((scalar.hard - hard_sum).abs() < 1e-9, "hard {} != record sum {}", scalar.hard, hard_sum);
        assert!((scalar.soft - soft_sum).abs() < 1e-9, "soft {} != record sum {}", scalar.soft, soft_sum);
        assert!(scalar.hard >= 10.0);
    }

    /// Mutation targeting sees exactly the assignments named by hard records.
    #[test]
    fn conflicted_indices_match_hard_records() {
        use crate::scheduler::conflicts::{distinct_hard_conflicts, RecordSink};
        let (cfg, sched) = sf3pl();
        let params = SolverParams::default();
        let mut e = FastEvaluator::new(&cfg, &params);

        let indices = e.get_conflicted_indices(&sched);

        let mut rec = RecordSink::default();
        e.evaluate(&sched, &mut rec);
        let mut expected: Vec<usize> = distinct_hard_conflicts(&rec.records)
            .iter()
            .flat_map(|c| c.who.clone())
            .collect();
        expected.sort_unstable();
        expected.dedup();

        assert_eq!(indices, expected);
        assert_eq!(indices, vec![0, 1], "both stage-ordered matches should be flagged");
    }

    /// Randomised property test for the unification invariant. The two tests
    /// above check one hand-built conflict; this one scores hundreds of random
    /// (mostly-broken) schedules over a config that exercises volunteers,
    /// interviews, multiple fields and break rules, and asserts on every one:
    ///   * scalar hard cost == sum of hard record weights,
    ///   * scalar soft cost == sum of soft record weights,
    ///   * conflicted indices == the distinct assignments named by hard records.
    /// This is the guardrail behind the "cost and conflicts can never drift
    /// apart" claim — and behind the incremental evaluator when it lands.
    #[test]
    fn sinks_agree_on_random_schedules() {
        use crate::scheduler::conflicts::{distinct_hard_conflicts, CostClass, RecordSink, ScalarSink};
        use crate::scheduler::internal::{InternalAssignment, InternalSchedule};
        use rand::{Rng, SeedableRng, rngs::StdRng};

        // A config broad enough to fire most hard rule kinds.
        let mut config = TournamentConfig::default();
        config.divisions = vec![
            Division {
                id: "d1".into(), name: "D1".into(), mode: SchedulingMode::HeadToHead,
                games_per_team: 2, volunteers_required: 1, duration_minutes: 20,
                allowed_fields: None, interviews_enabled: true,
                interview_volunteers_required: 1, interview_duration_minutes: 10,
                finals_enabled: true, finals_rounds: Some(FinalsRounds::Semis), finals_duration_minutes: Some(20),
                finals_third_place_playoff: true, color: None, min_match_break_minutes: Some(30),
            },
            Division {
                id: "d2".into(), name: "D2".into(), mode: SchedulingMode::HeadToHead,
                games_per_team: 2, volunteers_required: 1, duration_minutes: 20,
                allowed_fields: None, interviews_enabled: false,
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
            },
        ];
        for (d, names) in [("d1", ["A", "B", "C", "D"]), ("d2", ["E", "F", "G", "H"])] {
            for t in names {
                config.teams.push(Team { name: t.into(), division_id: d.into(), organization: format!("org{t}") });
            }
        }
        config.fields = vec![
            Field { id: "c1".into(), name: "Court 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "c2".into(), name: "Court 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["d1".into()]) },
            Field { id: "iv".into(), name: "Interview Room".into(), kind: FieldKind::Interview, allowed_divisions: None },
        ];
        // Interleaved competition + interview slots across one day.
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        config.time_slots = (0..12u32)
            .map(|i| {
                let start = 9 * 60 + i * 15;
                let kind = if i % 4 == 3 { FieldKind::Interview } else { FieldKind::Competition };
                TimeSlot { id: format!("s{i}"), day: "Sat".into(), start_time: fmt(start), end_time: fmt(start + 15), kind }
            })
            .collect();
        config.day_configs = vec![DayGenConfig { day: "Sat".into(), ..Default::default() }];
        config.volunteers = vec![
            Volunteer { id: "v1".into(), name: "V1".into(), availabilities: vec![], capabilities: None, conflict_organizations: vec![], attendance_status: Default::default() },
            Volunteer { id: "v2".into(), name: "V2".into(), availabilities: vec!["s0".into(), "s1".into()], capabilities: Some(vec!["d1".into()]), conflict_organizations: vec!["orgA".into()], attendance_status: Default::default() },
            Volunteer { id: "v3".into(), name: "V3".into(), availabilities: vec![], capabilities: Some(vec!["d2".into()]), conflict_organizations: vec![], attendance_status: Default::default() },
        ];

        let activities = crate::scheduler::generate_activities(&config);
        assert!(!activities.is_empty());
        let internal = InternalTournamentConfig::compile(&config, &activities);

        // Params turned up so as many hard rule kinds as possible can fire.
        let params = SolverParams {
            field_variety_strict: true,
            vol_daily_shift_cap: 2,
            team_min_break_minutes: 20,
            team_match_min_break_minutes: 30,
            ..SolverParams::default()
        };

        let (n_slots, n_fields, n_vols, n_acts) =
            (internal.slots.len(), internal.fields.len(), internal.volunteers.len(), internal.activities.len());
        let mut e = FastEvaluator::new(&internal, &params);
        let mut rng = StdRng::seed_from_u64(0xA11CE);

        for _ in 0..300 {
            let assignments = (0..n_acts)
                .map(|_| InternalAssignment {
                    slot_idx: rng.gen_range(0..n_slots),
                    field_idx: if rng.gen_bool(0.85) { Some(rng.gen_range(0..n_fields)) } else { None },
                    volunteer_indices: (0..n_vols).filter(|_| rng.gen_bool(0.4)).collect(),
                })
                .collect();
            let sched = InternalSchedule { assignments };

            let mut scalar = ScalarSink::default();
            e.evaluate(&sched, &mut scalar);
            let mut rec = RecordSink::default();
            e.evaluate(&sched, &mut rec);
            let mut conflicted = e.get_conflicted_indices(&sched);
            conflicted.sort_unstable();

            let hard_sum: f64 = rec.records.iter().filter(|c| c.class == CostClass::Hard).map(|c| c.weight).sum();
            let soft_sum: f64 = rec.records.iter().filter(|c| c.class == CostClass::Soft).map(|c| c.weight).sum();
            assert!((scalar.hard - hard_sum).abs() < 1e-6, "hard {} != hard record sum {}", scalar.hard, hard_sum);
            assert!((scalar.soft - soft_sum).abs() < 1e-6, "soft {} != soft record sum {}", scalar.soft, soft_sum);

            let mut expected: Vec<usize> =
                distinct_hard_conflicts(&rec.records).iter().flat_map(|c| c.who.clone()).collect();
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(conflicted, expected, "conflicted indices disagree with hard records");
        }
    }

    /// An interview immediately followed by a match for the same team (the
    /// reported 14:12 interview → 14:20 game case) is a hard conflict under the
    /// default minimum break, and the gap is measured in real minutes — not slot
    /// indices — so it fires regardless of how interview/competition slots interleave.
    #[test]
    fn interview_then_match_violates_minimum_break() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![Division {
            id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 1, volunteers_required: 0, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: true,
            interview_volunteers_required: 0, interview_duration_minutes: 8,
            finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
            finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        }];
        // Interview 14:12–14:20 then a match starting exactly at 14:20 → 0-minute gap.
        config.time_slots = vec![
            TimeSlot { id: "i1".into(), day: "Sat".into(), start_time: "14:12".into(), end_time: "14:20".into(), kind: FieldKind::Interview },
            TimeSlot { id: "c1".into(), day: "Sat".into(), start_time: "14:20".into(), end_time: "14:40".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "tbl".into(), name: "Table B".into(), kind: FieldKind::Interview, allowed_divisions: None },
            Field { id: "fld".into(), name: "Field 7".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![DayGenConfig { day: "Sat".into(), interviews_enabled: true, ..Default::default() }];

        let activities = vec![
            Activity::Interview { id: "iv".into(), team: "T1".into(), division_id: "div1".into(), duration_minutes: 8 },
            Activity::Match { id: "m".into(), team_a: "T1".into(), team_b: "T2".into(), division_id: "div1".into(), duration_minutes: 20, stage: MatchStage::RoundRobin { cycle: 0, round: 0 } },
        ];
        let internal_config = InternalTournamentConfig::compile(&config, &activities);
        let schedule = crate::scheduler::internal::InternalSchedule {
            assignments: vec![
                crate::scheduler::internal::InternalAssignment { slot_idx: 0, field_idx: Some(0), volunteer_indices: vec![] },
                crate::scheduler::internal::InternalAssignment { slot_idx: 1, field_idx: Some(1), volunteer_indices: vec![] },
            ],
        };

        // Default: a 10-minute minimum break ⇒ the 0-minute gap is a hard conflict.
        let params = SolverParams::default();
        let mut e = FastEvaluator::new(&internal_config, &params);
        let mut rec = RecordSink::default();
        e.evaluate(&schedule, &mut rec);
        let breaks: Vec<_> = rec.records.iter()
            .filter(|c| matches!(c.kind, ConflictKind::TeamMinBreak { .. }))
            .collect();
        assert_eq!(breaks.len(), 1, "expected one minimum-break hard conflict, got {:?}", rec.records);
        assert_eq!(breaks[0].class, CostClass::Hard);

        // Mutation targeting (hard-only sink) must also see it.
        assert!(e.get_conflicted_indices(&schedule).contains(&1));

        // Disabling the floor removes the hard conflict (the gap becomes soft-only).
        let off = SolverParams { team_min_break_minutes: 0, ..SolverParams::default() };
        let mut e_off = FastEvaluator::new(&internal_config, &off);
        let (hard, _soft) = e_off.calculate_total_cost(&schedule);
        assert_eq!(hard, 0.0, "no minimum break ⇒ no hard conflict");
    }

    /// Two of the same team's matches scheduled with only a 5-minute gap violate
    /// the default recharge break (hard), and a per-division override of 0 turns
    /// it off just for that division.
    #[test]
    fn consecutive_matches_violate_recharge_break() {
        let make = |div_break: Option<u32>| {
            let mut config = TournamentConfig::default();
            config.divisions = vec![Division {
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
                games_per_team: 2, volunteers_required: 0, duration_minutes: 20,
                allowed_fields: None, interviews_enabled: false,
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false, color: None, min_match_break_minutes: div_break,
            }];
            // 09:00–09:20 then 09:25–09:45 → a 5-minute gap for team T1.
            config.time_slots = vec![
                TimeSlot { id: "c1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
                TimeSlot { id: "c2".into(), day: "Sat".into(), start_time: "09:25".into(), end_time: "09:45".into(), kind: FieldKind::Competition },
            ];
            config.fields = vec![
                Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
                Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
            ];
            config.day_configs = vec![DayGenConfig { day: "Sat".into(), ..Default::default() }];

            let activities = vec![
                Activity::Match { id: "m1".into(), team_a: "T1".into(), team_b: "T2".into(), division_id: "div1".into(), duration_minutes: 20, stage: MatchStage::RoundRobin { cycle: 0, round: 0 } },
                Activity::Match { id: "m2".into(), team_a: "T1".into(), team_b: "T3".into(), division_id: "div1".into(), duration_minutes: 20, stage: MatchStage::RoundRobin { cycle: 0, round: 1 } },
            ];
            let internal_config = InternalTournamentConfig::compile(&config, &activities);
            let schedule = crate::scheduler::internal::InternalSchedule {
                assignments: vec![
                    crate::scheduler::internal::InternalAssignment { slot_idx: 0, field_idx: Some(0), volunteer_indices: vec![] },
                    crate::scheduler::internal::InternalAssignment { slot_idx: 1, field_idx: Some(1), volunteer_indices: vec![] },
                ],
            };
            (internal_config, schedule)
        };

        // Default 10-minute floor, no division override ⇒ the 5-minute gap is hard.
        let (cfg, sched) = make(None);
        let params = SolverParams::default();
        let mut e = FastEvaluator::new(&cfg, &params);
        let mut rec = RecordSink::default();
        e.evaluate(&sched, &mut rec);
        let breaks: Vec<_> = rec.records.iter()
            .filter(|c| matches!(c.kind, ConflictKind::TeamMatchBreak { .. }))
            .collect();
        assert_eq!(breaks.len(), 1, "expected one recharge-break hard conflict, got {:?}", rec.records);
        assert_eq!(breaks[0].class, CostClass::Hard);
        assert!(e.get_conflicted_indices(&sched).contains(&0) && e.get_conflicted_indices(&sched).contains(&1));

        // A per-division override of 0 disables the recharge floor for this division.
        let (cfg_off, sched_off) = make(Some(0));
        let mut e_off = FastEvaluator::new(&cfg_off, &params);
        let (hard, _soft) = e_off.calculate_total_cost(&sched_off);
        assert_eq!(hard, 0.0, "division override of 0 ⇒ no recharge hard conflict");
    }

    /// A volunteer marked no-show for a day is unavailable that day, so the
    /// solver and the diagnostics both see the conflict.
    #[test]
    fn no_show_volunteer_is_treated_as_unavailable() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![Division {
            id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 1, volunteers_required: 1, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: false,
            interview_volunteers_required: 0, interview_duration_minutes: 0,
            finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
            finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        }];
        config.teams = vec![
            Team { name: "A".into(), division_id: "div1".into(), organization: "OrgA".into() },
            Team { name: "B".into(), division_id: "div1".into(), organization: "OrgB".into() },
        ];
        config.fields = vec![Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None }];
        config.time_slots = vec![TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition }];
        config.day_configs = vec![DayGenConfig { day: "Sat".into(), ..Default::default() }];

        let mut vol = Volunteer {
            id: "v1".into(), name: "Vol One".into(), availabilities: vec!["s1".into()],
            capabilities: None, conflict_organizations: vec![], attendance_status: Default::default(),
        };
        vol.attendance_status.insert("Sat".into(), AttendanceStatus::NoShow);
        config.volunteers = vec![vol];

        let schedule = Schedule {
            assignments: vec![ScheduleAssignment {
                activity: Activity::Match { id: "div1_m".into(), team_a: "A".into(), team_b: "B".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } },
                time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec!["v1".into()],
            }],
        };

        let params = SolverParams::default();
        let (hard, _soft) = evaluate_schedule_cost(&config, &schedule, &params);
        assert!(hard > 0.0, "assigning a no-show volunteer must be a hard conflict");

        let conflicts = crate::scheduler::get_schedule_conflicts(&config, &schedule, &params);
        assert!(conflicts.iter().any(|c| c.contains("not available")), "expected an availability conflict, got {conflicts:?}");
    }
}
