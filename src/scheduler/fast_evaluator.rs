use super::internal::{InternalTournamentConfig, InternalAssignment, InternalSchedule};
use super::SolverParams;
use crate::model::{FairnessMode, SpecialistMode, FieldKind, Schedule, SchedulingMode, TournamentConfig};
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

    pub fn calculate_total_cost(&mut self, schedule: &InternalSchedule) -> (f64, f64) {
        let mut hard = 0.0;
        let mut soft = 0.0;

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
                hard += 1.0 * multiplier;
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
                if !suitable { hard += 1.0 * multiplier; }
                
                self.field_total_counts[f_idx] += 1;
                if activity.is_interview { self.field_interview_counts[f_idx] += 1; }
                else { self.field_match_counts[f_idx] += 1; }
            } else {
                // Hard penalty for not having a field assigned
                hard += 1.0 * multiplier;
            }

            // Volunteer
            let overlapped = &self.config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
            for &v_idx in &assign.volunteer_indices {
                let v = &self.config.volunteers[v_idx];
                self.volunteer_shift_counts[v_idx] += 1;
                let day_idx = self.config.slots[assign.slot_idx].day_idx;
                self.volunteer_daily_counts[v_idx][day_idx] += 1;

                for &s_idx in overlapped {
                    if !v.availability_slots[s_idx] { hard += 1.0 * multiplier; }
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
                    if self.config.strict_capabilities || activity.is_interview { hard += 1.0 * multiplier; }
                    else { soft += self.params.vol_capability_weight * multiplier; }
                }

                for &t_idx in &activity.team_indices {
                    let team = &self.config.teams[t_idx];
                    if v.conflict_org_indices.contains(&team.org_idx) { hard += 1.0 * multiplier; }
                }
            }

            let req = if activity.is_interview { self.config.divisions[activity.division_idx].interview_volunteers_required }
                      else { self.config.divisions[activity.division_idx].volunteers_required };
            if assign.volunteer_indices.len() < req {
                hard += (req - assign.volunteer_indices.len()) as f64 * multiplier;
            }

            let last_slot = &self.config.slots[assign.slot_idx];
            if activity.is_interview && !self.config.day_interviews_enabled[last_slot.day_idx] {
                hard += 10.0 * multiplier;
            }
            let day_end = self.config.slots.iter().filter(|s| s.day_idx == last_slot.day_idx)
                .map(|s| s.start_minutes + s.duration_minutes).max().unwrap_or(0);
            if last_slot.start_minutes + activity.duration_minutes > day_end { hard += 1.0 * multiplier; }

            if activity.is_interview && self.params.interview_late_weight > 0.0 {
                soft += (assign.slot_idx as f64) * self.params.interview_late_weight;
            }
        }

        // Double Booking Hard Conflicts
        for team in &self.team_slot_occupancy { for &count in team { if count > 1 { hard += (count - 1) as f64; } } }
        for field in &self.field_slot_occupancy { for &count in field { if count > 1 { hard += (count - 1) as f64; } } }
        for vol in &self.volunteer_slot_occupancy { for &count in vol { if count > 1 { hard += (count - 1) as f64; } } }

        // Daily shift cap
        if self.params.vol_daily_shift_cap > 0 {
            for vol in &self.volunteer_daily_counts {
                for &count in vol {
                    if count > self.params.vol_daily_shift_cap as u32 {
                        hard += (count - self.params.vol_daily_shift_cap as u32) as f64;
                    }
                }
            }
        }

        // Grouped soft penalties
        for t_idx in 0..self.config.teams.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.team_day_assignments[t_idx][d_idx];
                list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
                soft += Self::calculate_team_day_penalties(self.params, self.config, schedule, list);
            }
        }
        for v_idx in 0..self.config.volunteers.len() {
            for d_idx in 0..self.config.days.len() {
                let list = &mut self.vol_day_assignments[v_idx][d_idx];
                list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
                soft += Self::calculate_vol_day_penalties(self.params, self.config, schedule, list);
            }
        }
        for div_idx in 0..self.config.divisions.len() {
            let list = &mut self.division_assignments[div_idx];
            list.sort_by_key(|&idx| schedule.assignments[idx].slot_idx);
            let (h, s) = Self::calculate_division_penalties(self.params, self.config, schedule, list);
            hard += h;
            soft += s;
        }

        // Variance soft penalties
        soft += calculate_variance(&self.field_match_counts) * self.params.field_balance_weight;
        soft += calculate_variance(&self.field_interview_counts) * self.params.field_balance_weight;
        soft += calculate_variance(&self.field_total_counts) * (self.params.field_balance_weight * 0.5);

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
            soft += var * weight;
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
                    soft += (count - 1) as f64 * weight;
                }
            }
        }

        // Field variety: penalise a team being assigned to the same field
        // repeatedly. Only tracked under strict mode (all divisions) or for
        // IndividualRun divisions, mirroring the original evaluator.
        if self.params.field_variety_strict || self.config.divisions.iter().any(|d| d.mode == SchedulingMode::IndividualRun) {
            let mut team_field_counts: HashMap<(usize, usize), u32> = HashMap::new();
            for (idx, assign) in schedule.assignments.iter().enumerate() {
                let Some(f_idx) = assign.field_idx else { continue };
                let activity = &self.config.activities[idx];
                let div_mode = self.config.divisions[activity.division_idx].mode;
                if !(self.params.field_variety_strict || div_mode == SchedulingMode::IndividualRun) {
                    continue;
                }
                for &t_idx in &activity.team_indices {
                    *team_field_counts.entry((t_idx, f_idx)).or_insert(0) += 1;
                }
            }
            for &count in team_field_counts.values() {
                if count > 1 {
                    if self.params.field_variety_strict {
                        hard += (count - 1) as f64;
                    } else {
                        soft += (count - 1) as f64 * self.params.field_variety_weight;
                    }
                }
            }
        }

        // Peak period: encourage an even spread of activities across time slots
        // by penalising the variance of per-slot occupancy.
        if self.params.peak_period_weight > 0.0 {
            let mut slot_counts = vec![0.0f64; self.config.slots.len()];
            for (idx, assign) in schedule.assignments.iter().enumerate() {
                let activity = &self.config.activities[idx];
                let overlapped = &self.config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
                for &s_idx in overlapped {
                    slot_counts[s_idx] += 1.0;
                }
            }
            let active: Vec<f64> = slot_counts.into_iter().filter(|&c| c > 0.0).collect();
            soft += calculate_variance_f64(&active) * self.params.peak_period_weight;
        }

        self.current_hard_conflicts = hard;
        self.current_soft_penalties = soft;
        (hard, soft)
    }

    pub fn get_conflicted_indices(&self, schedule: &InternalSchedule) -> Vec<usize> {
        let mut conflicted = vec![false; schedule.assignments.len()];

        for (idx, assign) in schedule.assignments.iter().enumerate() {
            let activity = &self.config.activities[idx];
            let buckets = &self.config.activity_buckets[assign.slot_idx][activity.duration_class];
            let slot = &self.config.slots[assign.slot_idx];

            if (activity.is_interview && slot.kind == FieldKind::Competition) || (!activity.is_interview && slot.kind == FieldKind::Interview) {
                conflicted[idx] = true;
            }

            // Occupancy Check
            for &b_idx in buckets {
                for &t_idx in &activity.team_indices {
                    if self.team_slot_occupancy[t_idx][b_idx] > 1 { conflicted[idx] = true; }
                }
                if let Some(f_idx) = assign.field_idx
                    && self.field_slot_occupancy[f_idx][b_idx] > 1 { conflicted[idx] = true; }
                for &v_idx in &assign.volunteer_indices {
                    if self.volunteer_slot_occupancy[v_idx][b_idx] > 1 { conflicted[idx] = true; }
                }
            }

            // Field Suitability & Presence
            if let Some(f_idx) = assign.field_idx {
                let f = &self.config.fields[f_idx];
                let mut suitable = true;
                if f.kind == FieldKind::Competition && activity.is_interview { suitable = false; }
                if f.kind == FieldKind::Interview && !activity.is_interview { suitable = false; }
                if let Some(ref allowed) = f.allowed_division_indices
                    && !allowed.contains(&activity.division_idx) { suitable = false; }
                if !suitable { conflicted[idx] = true; }
            } else {
                conflicted[idx] = true;
            }

            // Volunteer
            let overlapped = &self.config.activity_overlapping_slots[assign.slot_idx][activity.duration_class];
            for &v_idx in &assign.volunteer_indices {
                let v = &self.config.volunteers[v_idx];
                let day_idx = self.config.slots[assign.slot_idx].day_idx;

                for &s_idx in overlapped {
                    if !v.availability_slots[s_idx] { conflicted[idx] = true; }
                }

                let mut qualified = true;
                if activity.is_interview {
                    if !self.config.can_interview[v_idx] {
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

                if !qualified && (self.config.strict_capabilities || activity.is_interview) {
                    conflicted[idx] = true;
                }

                for &t_idx in &activity.team_indices {
                    let team = &self.config.teams[t_idx];
                    if v.conflict_org_indices.contains(&team.org_idx) { conflicted[idx] = true; }
                }

                if self.params.vol_daily_shift_cap > 0 && self.volunteer_daily_counts[v_idx][day_idx] > self.params.vol_daily_shift_cap as u32 {
                    conflicted[idx] = true;
                }
            }

            let req = if activity.is_interview { self.config.divisions[activity.division_idx].interview_volunteers_required }
                      else { self.config.divisions[activity.division_idx].volunteers_required };
            if assign.volunteer_indices.len() < req {
                conflicted[idx] = true;
            }

            let last_slot = &self.config.slots[assign.slot_idx];
            if activity.is_interview && !self.config.day_interviews_enabled[last_slot.day_idx] {
                conflicted[idx] = true;
            }

            let day_end = self.config.slots.iter().filter(|s| s.day_idx == last_slot.day_idx)
                .map(|s| s.start_minutes + s.duration_minutes).max().unwrap_or(0);
            if last_slot.start_minutes + activity.duration_minutes > day_end {
                conflicted[idx] = true;
            }
        }

        conflicted.iter().enumerate().filter(|&(_, &c)| c).map(|(i, _)| i).collect()
    }

    fn calculate_team_day_penalties(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize]) -> f64 {
        if list.is_empty() { return 0.0; }
        let mut soft = 0.0;

        // Wait Time
        if params.team_wait_time_weight > 0.0 && list.len() > 1 {
            let non_interviews: Vec<usize> = list.iter().filter(|&&i| !config.activities[i].is_interview).copied().collect();
            if non_interviews.len() > 1 {
                let first = non_interviews[0];
                let last = *non_interviews.last().unwrap();
                let span = schedule.assignments[last].slot_idx - schedule.assignments[first].slot_idx;
                let excessive_span = span.saturating_sub(non_interviews.len() * 2);
                soft += (excessive_span as f64) * params.team_wait_time_weight;
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
                    soft += params.team_back_to_back_weight;
                }
            }

            // Interview gap (scan all other activities for this team on this day)
            if params.interview_match_gap_weight > 0.0 {
                for &idx2 in list.iter().skip(i + 1) {
                    let act2 = &config.activities[idx2];
                    if act1.is_interview != act2.is_interview {
                        let assign2 = &schedule.assignments[idx2];
                        let gap = assign2.slot_idx.abs_diff(assign1.slot_idx);
                        if gap < 2 { soft += params.interview_match_gap_weight; }
                    }
                }
            }
        }
        soft
    }

    fn calculate_vol_day_penalties(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize]) -> f64 {
        if list.len() < 2 { return 0.0; }
        let mut soft = 0.0;
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
                soft += params.vol_consecutive_weight;
                if params.vol_travel_weight > 0.0 && assign1.field_idx != assign2.field_idx {
                    soft += params.vol_travel_weight;
                }
            }
        }
        soft
    }

    fn calculate_division_penalties(params: &SolverParams, config: &InternalTournamentConfig, schedule: &InternalSchedule, list: &[usize]) -> (f64, f64) {
        if list.is_empty() { return (0.0, 0.0); }
        let mut hard = 0.0;
        let mut soft = 0.0;
        
        if params.round_order_weight < 0.0 { return (0.0, 0.0); }

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
                            hard += 10.0;
                        } else {
                            // stage1 < stage2: check for overlap
                            if slot1.day_idx == slot2.day_idx && end1 > slot2.start_minutes {
                                // Overlap between different stages.
                                hard += 10.0;
                            }
                        }
                    }
                } else if params.round_order_weight > 0.0 {
                    // Same stage: enforce round order with soft penalty
                    
                    // Same day check
                    if slot1.day_idx != slot2.day_idx {
                        if round1 > round2 {
                            soft += params.round_order_weight;
                        }
                        continue;
                    }

                    if round1 > round2 {
                        // Strictly out of order
                        soft += params.round_order_weight;
                    } else if round1 < round2
                        && end1 > slot2.start_minutes && assign1.slot_idx == assign2.slot_idx {
                            // Traditional same-start overlap penalty for same-stage activities
                            soft += params.round_order_weight * 0.5;
                        }
                }
            }
        }
        (hard, soft)
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
}
