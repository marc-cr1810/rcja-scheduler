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
            let activity = &self.config.activities[idx];
            let day_idx = self.config.slots[assign.slot_idx].day_idx;

            // Rebuild grouped lists
            for &t_idx in &activity.team_indices {
                self.team_day_assignments[t_idx][day_idx].push(idx);
            }
            for &v_idx in &assign.volunteer_indices {
                self.vol_day_assignments[v_idx][day_idx].push(idx);
            }
            self.division_assignments[activity.division_idx].push(idx);

            let multiplier = if activity.is_final { self.params.finals_priority_multiplier } else { 1.0 };
            let buckets = &self.config.activity_buckets[assign.slot_idx][activity.duration_class];
            let slot = &self.config.slots[assign.slot_idx];

            if (activity.is_interview && slot.kind == FieldKind::Competition) || (!activity.is_interview && slot.kind == FieldKind::Interview) {
                sink.report(CostClass::Hard, multiplier, ConflictKind::SlotKindMismatch, &[idx]);
            }

            // Occupancy
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

            // Field Suitability
            if let Some(f_idx) = assign.field_idx {
                let f = &self.config.fields[f_idx];
                let mut suitable = true;
                if f.kind == FieldKind::Competition && activity.is_interview { suitable = false; }
                if f.kind == FieldKind::Interview && !activity.is_interview { suitable = false; }
                if let Some(ref allowed) = f.allowed_division_indices
                    && !allowed.contains(&activity.division_idx) { suitable = false; }
                if !suitable {
                    sink.report(CostClass::Hard, multiplier, ConflictKind::FieldUnsuitable { field_idx: f_idx }, &[idx]);
                }

                self.field_total_counts[f_idx] += 1;
                if activity.is_interview { self.field_interview_counts[f_idx] += 1; }
                else { self.field_match_counts[f_idx] += 1; }
            } else {
                sink.report(CostClass::Hard, multiplier, ConflictKind::FieldMissing, &[idx]);
            }

            // Volunteer
            let overlapped = &self.config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
            for &v_idx in &assign.volunteer_indices {
                let v = &self.config.volunteers[v_idx];
                self.volunteer_shift_counts[v_idx] += 1;
                self.volunteer_daily_counts[v_idx][day_idx] += 1;

                for &s_idx in overlapped {
                    if !v.availability_slots[s_idx] {
                        sink.report(CostClass::Hard, multiplier, ConflictKind::VolUnavailable { vol_idx: v_idx, slot_idx: s_idx }, &[idx]);
                    }
                }

                let mut qualified = true;
                if activity.is_interview {
                    if !self.config.can_interview[v_idx] {
                        if let Some(ref caps) = v.capability_indices {
                            if !caps.contains(&activity.division_idx) { qualified = false; }
                        } else {
                            // If caps is None, they are qualified for everything
                            qualified = true;
                        }
                    } else {
                        qualified = true;
                    }
                } else if let Some(ref caps) = v.capability_indices
                && !caps.contains(&activity.division_idx) { qualified = false; }

                if !qualified {
                    if self.config.strict_capabilities || activity.is_interview {
                        sink.report(CostClass::Hard, multiplier, ConflictKind::VolUnqualified { vol_idx: v_idx }, &[idx]);
                    } else {
                        sink.report(CostClass::Soft, self.params.vol_capability_weight * multiplier, ConflictKind::VolCapabilitySoft { vol_idx: v_idx }, &[idx]);
                    }
                }

                for &t_idx in &activity.team_indices {
                    let team = &self.config.teams[t_idx];
                    if v.conflict_org_indices.contains(&team.org_idx) {
                        sink.report(CostClass::Hard, multiplier, ConflictKind::ConflictOfInterest { vol_idx: v_idx, team_idx: t_idx }, &[idx]);
                    }
                }
            }

            let req = if activity.is_interview { self.config.divisions[activity.division_idx].interview_volunteers_required }
                      else { self.config.divisions[activity.division_idx].volunteers_required };
            if assign.volunteer_indices.len() < req {
                let missing = (req - assign.volunteer_indices.len()) as f64;
                sink.report(CostClass::Hard, missing * multiplier, ConflictKind::UnderRostered { required: req, assigned: assign.volunteer_indices.len() }, &[idx]);
            }

            if activity.is_interview && !self.config.day_interviews_enabled[slot.day_idx] {
                sink.report(CostClass::Hard, 10.0 * multiplier, ConflictKind::InterviewsDisabled, &[idx]);
            }
            let day_end = self.config.slots.iter().filter(|s| s.day_idx == slot.day_idx)
                .map(|s| s.start_minutes + s.duration_minutes).max().unwrap_or(0);
            if slot.start_minutes + activity.duration_minutes > day_end {
                sink.report(CostClass::Hard, multiplier, ConflictKind::DurationExceedsDay, &[idx]);
            }

            if activity.is_interview && self.params.interview_late_weight > 0.0 {
                sink.report(CostClass::Soft, (assign.slot_idx as f64) * self.params.interview_late_weight, ConflictKind::InterviewLate, &[idx]);
            }
        }

        // Double booking + daily shift cap (hard), derived from the occupancy
        // state populated above.
        self.report_occupancy(schedule, sink);

        // Division stage ordering (hard) must always run so mutation targeting
        // can find it; round-order (soft) rides along in the same pass and is a
        // no-op for sinks that ignore soft.
        for div_idx in 0..self.config.divisions.len() {
            let list = &mut self.division_assignments[div_idx];
            list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
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

        // Everything below is purely soft; sinks that only want hard conflicts
        // (mutation targeting) skip it entirely.
        if !want_soft {
            return;
        }

        // Grouped soft penalties
        for t_idx in 0..self.config.teams.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.team_day_assignments[t_idx][d_idx];
                list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
                Self::report_team_day_penalties(self.params, self.config, schedule, list, sink);
            }
        }
        for v_idx in 0..self.config.volunteers.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.vol_day_assignments[v_idx][d_idx];
                list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
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
        let mut sink = ConflictedSink::new(schedule.assignments.len());
        self.evaluate(schedule, &mut sink);
        sink.into_indices()
    }

    fn report_team_day_penalties<S: ConflictSink>(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize], sink: &mut S) {
        if list.is_empty() { return; }

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

        // Back-to-back and Interview-Match Gap
        for i in 0..list.len() {
            let idx1 = list[i];
            let assign1 = &schedule.assignments[idx1];
            let act1 = &config.activities[idx1];

            if i + 1 < list.len() {
                let idx2 = list[i+1];
                let assign2 = &schedule.assignments[idx2];

                // Back-to-back: the next activity starts exactly when this one
                // ends (same day; these lists are per-day). Compared in minutes
                // so it stays correct with non-uniform slot durations.
                let slot1 = &config.slots[assign1.slot_idx];
                let slot2 = &config.slots[assign2.slot_idx];
                if slot1.day_idx == slot2.day_idx
                    && slot2.start_minutes == slot1.start_minutes + act1.duration_minutes {
                    sink.report(CostClass::Soft, params.team_back_to_back_weight, ConflictKind::TeamBackToBack, &[idx1, idx2]);
                }
            }

            // Interview gap (scan all other activities for this team on this day)
            if params.interview_match_gap_weight > 0.0 {
                for &idx2 in list.iter().skip(i + 1) {
                    let act2 = &config.activities[idx2];
                    if act1.is_interview != act2.is_interview {
                        let assign2 = &schedule.assignments[idx2];
                        let gap = assign2.slot_idx.abs_diff(assign1.slot_idx);
                        if gap < 2 {
                            sink.report(CostClass::Soft, params.interview_match_gap_weight, ConflictKind::InterviewMatchGap, &[idx1, idx2]);
                        }
                    }
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
                color: None,
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
            finals_third_place_playoff: true, color: None,
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
            finals_third_place_playoff: false, color: None,
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
