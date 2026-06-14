use crate::model::{
    Activity, FieldKind, SchedulingMode, TournamentConfig,
};
use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalActivity {
    pub id: String,
    pub team_indices: Vec<usize>,
    pub division_idx: usize,
    pub duration_minutes: u32,
    /// Index into the distinct-durations table, used to look up precomputed
    /// bucket/overlap data by `[slot_idx][duration_class]` without hashing.
    pub duration_class: usize,
    pub stage: usize,
    pub round_index: usize,
    pub is_final: bool,
    pub is_interview: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalVolunteer {
    pub id: String,
    pub availability_slots: Vec<bool>, // indexed by slot_idx
    pub capability_indices: Option<Vec<usize>>, // None means all
    pub conflict_org_indices: Vec<usize>,
    /// Field indices this volunteer is locked to. `None` (or empty) means no
    /// restriction; otherwise the volunteer may only be rostered on a listed field.
    pub locked_field_indices: Option<Vec<usize>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalField {
    pub id: String,
    pub kind: FieldKind,
    pub allowed_division_indices: Option<Vec<usize>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalDivision {
    pub id: String,
    pub mode: SchedulingMode,
    pub volunteers_required: usize,
    pub interview_volunteers_required: usize,
    /// Per-division override for the minimum match recharge break (minutes).
    /// `None` means inherit the global solver setting.
    pub min_match_break_minutes: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalTeam {
    pub name: String,
    pub org_idx: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalSlot {
    pub id: String,
    pub day_idx: usize,
    pub start_minutes: u32,
    pub duration_minutes: u32,
    pub kind: FieldKind,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InternalTournamentConfig {
    pub activities: Vec<InternalActivity>,
    pub volunteers: Vec<InternalVolunteer>,
    pub fields: Vec<InternalField>,
    pub slots: Vec<InternalSlot>,
    pub divisions: Vec<InternalDivision>,
    pub teams: Vec<InternalTeam>,
    pub organizations: Vec<String>,
    pub days: Vec<String>,
    pub day_interviews_enabled: Vec<bool>,
    
    // [slot_idx][duration_class] -> list of global 5-minute bucket indices
    pub activity_buckets: Vec<Vec<Vec<usize>>>,
    // [slot_idx][duration_class] -> list of other slot indices that overlap
    pub activity_overlapping_slots: Vec<Vec<Vec<usize>>>,
    pub num_total_buckets: usize,

    /// Pre-computed chronological slot ranges for each round index.
    pub round_ranges: Vec<std::ops::Range<usize>>,
    
    pub strict_capabilities: bool,

    // NEW: track if a volunteer has the "Interview" capability
    pub can_interview: Vec<bool>,
}

#[derive(Debug, Clone)]
pub struct InternalAssignment {
    pub slot_idx: usize,
    pub field_idx: Option<usize>,
    pub volunteer_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct InternalSchedule {
    pub assignments: Vec<InternalAssignment>,
}

impl InternalTournamentConfig {
    pub fn compile(config: &TournamentConfig, activities: &[Activity]) -> Self {
        let mut organizations = Vec::new();
        let mut org_map = HashMap::new();
        for team in &config.teams {
            if !org_map.contains_key(&team.organization) {
                org_map.insert(team.organization.clone(), organizations.len());
                organizations.push(team.organization.clone());
            }
        }

        let mut day_names = Vec::new();
        let mut day_to_idx = HashMap::new();
        let mut day_interviews_enabled = Vec::new();
        
        for dc in &config.day_configs {
            let day = dc.day.to_lowercase();
            if !day_to_idx.contains_key(&day) {
                day_to_idx.insert(day.clone(), day_names.len());
                day_names.push(day);
                day_interviews_enabled.push(dc.interviews_enabled);
            }
        }
        
        for slot in &config.time_slots {
            let day = slot.day.to_lowercase();
            if !day_to_idx.contains_key(&day) {
                day_to_idx.insert(day.clone(), day_names.len());
                day_names.push(day);
                day_interviews_enabled.push(true);
            }
        }

        let get_day_idx = |day: &str| {
            *day_to_idx.get(&day.to_lowercase()).unwrap_or(&99)
        };

        let mut sorted_slots = config.time_slots.clone();
        sorted_slots.sort_by_key(|s| (get_day_idx(&s.day), s.start_minutes()));

        let div_map: HashMap<String, usize> = config.divisions.iter().enumerate().map(|(i, d)| (d.id.clone(), i)).collect();
        // Internal field indices match the order of `config.fields`, so this map
        // resolves a volunteer's locked field IDs to those indices.
        let field_map: HashMap<String, usize> = config.fields.iter().enumerate().map(|(i, f)| (f.id.clone(), i)).collect();

        // Include ALL team names from activities, including finals placeholders
        let mut all_team_names: Vec<String> = config.teams.iter().map(|t| t.name.clone()).collect();
        for act in activities {
            for team in act.teams() {
                if !all_team_names.contains(&team.to_string()) {
                    all_team_names.push(team.to_string());
                }
            }
        }

        let mut internal_teams = Vec::new();
        let mut team_map = HashMap::new();
        for (i, name) in all_team_names.iter().enumerate() {
            team_map.insert(name.clone(), i);
            
            // If it's a real team, use its org, otherwise use a dummy "placeholder" org
            let org_idx = config.teams.iter()
                .find(|t| &t.name == name)
                .and_then(|t| org_map.get(&t.organization))
                .copied()
                .unwrap_or(999); // Dummy org index for placeholders

            internal_teams.push(InternalTeam {
                name: name.clone(),
                org_idx,
            });
        }

        let slot_id_to_internal_idx: HashMap<String, usize> = sorted_slots.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();

        let internal_slots: Vec<InternalSlot> = sorted_slots.iter().map(|s| InternalSlot {
            id: s.id.clone(),
            day_idx: get_day_idx(&s.day),
            start_minutes: s.start_minutes(),
            duration_minutes: s.duration_minutes(),
            kind: s.kind,
        }).collect();

        let internal_volunteers: Vec<InternalVolunteer> = config.volunteers.iter().map(|v| {
            let mut avail = vec![false; sorted_slots.len()];
            for s_id in &v.availabilities {
                if let Some(&idx) = slot_id_to_internal_idx.get(s_id) {
                    avail[idx] = true;
                }
            }
            // A volunteer marked as a no-show for a day is treated as unavailable
            // for every slot that day, so the solver and the diagnostics agree on
            // availability (a no-show used to be flagged only in the UI).
            for slot in &config.time_slots {
                if matches!(v.status_for_day(&slot.day), crate::model::AttendanceStatus::NoShow)
                    && let Some(&idx) = slot_id_to_internal_idx.get(&slot.id) {
                    avail[idx] = false;
                }
            }
            InternalVolunteer {
                id: v.id.clone(),
                availability_slots: avail,
                capability_indices: v.capabilities.as_ref().map(|caps| {
                    caps.iter().filter_map(|c| div_map.get(c).copied()).collect()
                }),
                conflict_org_indices: v.conflict_organizations.iter().filter_map(|o| org_map.get(o).copied()).collect(),
                locked_field_indices: v.locked_field_ids.as_ref().and_then(|ids| {
                    let idxs: Vec<usize> = ids.iter().filter_map(|f| field_map.get(f).copied()).collect();
                    if idxs.is_empty() { None } else { Some(idxs) }
                }),
            }
        }).collect();

        let can_interview: Vec<bool> = config.volunteers.iter().map(|v| {
            v.capabilities.as_ref().is_none_or(|caps| caps.contains(&"Interview".to_string()))
        }).collect();

        let internal_fields: Vec<InternalField> = config.fields.iter().map(|f| InternalField {
            id: f.id.clone(),
            kind: f.kind,
            allowed_division_indices: f.allowed_divisions.as_ref().map(|divs| {
                divs.iter().filter_map(|d| div_map.get(d).copied()).collect()
            }),
        }).collect();

        let internal_divisions: Vec<InternalDivision> = config.divisions.iter().map(|d| InternalDivision {
            id: d.id.clone(),
            mode: d.mode,
            volunteers_required: d.volunteers_required,
            interview_volunteers_required: d.interview_volunteers_required,
            min_match_break_minutes: d.min_match_break_minutes,
        }).collect();

        // Distinct activity durations. Each gets a "class" index so the solver
        // can look up precomputed bucket/overlap data by plain Vec indexing
        // (`[slot_idx][duration_class]`) instead of hashing a (slot, dur) key.
        let mut durations: Vec<u32> = activities.iter().map(|a| a.duration_minutes()).collect();
        durations.sort_unstable();
        durations.dedup();
        let dur_class: HashMap<u32, usize> = durations.iter().enumerate().map(|(i, &d)| (d, i)).collect();

        let internal_activities: Vec<InternalActivity> = activities.iter().map(|a| InternalActivity {
            id: a.id().to_string(),
            team_indices: a.teams().iter().filter_map(|t| team_map.get(*t).copied()).collect(),
            division_idx: *div_map.get(a.division_id()).unwrap(),
            duration_minutes: a.duration_minutes(),
            duration_class: dur_class[&a.duration_minutes()],
            stage: a.stage(),
            round_index: a.round_index(),
            is_final: a.is_final(),
            is_interview: matches!(a, Activity::Interview { .. }),
        }).collect();

        let bucket_size = 5u32;
        let day_span_minutes = 24 * 60;
        let buckets_per_day = day_span_minutes / bucket_size;

        let mut activity_buckets: Vec<Vec<Vec<usize>>> =
            vec![vec![Vec::new(); durations.len()]; internal_slots.len()];
        let mut activity_overlapping_slots: Vec<Vec<Vec<usize>>> =
            vec![vec![Vec::new(); durations.len()]; internal_slots.len()];

        for (slot_idx, slot) in internal_slots.iter().enumerate() {
            for (dc, &dur) in durations.iter().enumerate() {
                let start_min = slot.start_minutes;
                let end_min = start_min + dur;
                let day_offset = slot.day_idx as u32 * buckets_per_day;

                let mut buckets = Vec::new();
                let first_bucket = start_min / bucket_size;
                let last_bucket = (end_min - 1) / bucket_size;
                for b in first_bucket..=last_bucket {
                    buckets.push((day_offset + b) as usize);
                }
                activity_buckets[slot_idx][dc] = buckets;

                let mut overlapping = Vec::new();
                for (other_idx, other_slot) in internal_slots.iter().enumerate() {
                    if other_slot.day_idx == slot.day_idx {
                        let other_start = other_slot.start_minutes;
                        let other_end = other_start + other_slot.duration_minutes;
                        if start_min < other_end && other_start < end_min {
                            overlapping.push(other_idx);
                        }
                    }
                }
                activity_overlapping_slots[slot_idx][dc] = overlapping;
            }
        }

        let num_total_buckets = day_names.len() * buckets_per_day as usize;

        let n_slots = internal_slots.len();
        let mut round_ranges = Vec::new();
        if n_slots > 0 {
            let mut round_indices: Vec<usize> = internal_activities.iter()
                .filter(|a| !a.is_interview)
                .map(|a| a.round_index)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            round_indices.sort_unstable();

            if !round_indices.is_empty() {
                let max_round = *round_indices.last().unwrap();
                round_ranges = vec![0..n_slots; max_round + 1];

                // Separate RR rounds from Finals rounds
                let rr_rounds: Vec<usize> = round_indices.iter().cloned().filter(|&r| r < 10000).collect();
                let finals_rounds: Vec<usize> = round_indices.iter().cloned().filter(|&r| r >= 10000).collect();

                // Round windows are sized over *competition* slots only, in
                // chronological order. Interview slots are excluded because they
                // tend to cluster on a single day; counting them would inflate that
                // day's share of the index space and pull every round-robin game
                // onto it, leaving later days empty. Windowing over competition
                // columns instead lets round-robin spread across every day.
                let comp_indices: Vec<usize> = (0..n_slots)
                    .filter(|&i| internal_slots[i].kind == FieldKind::Competition)
                    .collect();
                let n_comp = comp_indices.len();

                if n_comp > 0 {
                    // Competition fields available to a division — used to size each
                    // band so a division never has more games in a band than it has
                    // columns x fields to hold them (which would force a double-booking).
                    let comp_fields_for_div = |div_idx: usize| -> usize {
                        internal_fields.iter().filter(|f| {
                            f.kind == FieldKind::Competition
                                && f.allowed_division_indices.as_ref().is_none_or(|a| a.contains(&div_idx))
                        }).count().max(1)
                    };
                    // Minimum competition columns a group of activities needs so that no
                    // single division overflows its available fields.
                    let min_cols = |pred: &dyn Fn(usize) -> bool| -> usize {
                        let mut per_div: HashMap<usize, usize> = HashMap::new();
                        for a in internal_activities.iter().filter(|a| !a.is_interview && pred(a.round_index)) {
                            *per_div.entry(a.division_idx).or_default() += 1;
                        }
                        per_div.iter()
                            .map(|(&d, &g)| g.div_ceil(comp_fields_for_div(d)))
                            .max().unwrap_or(1).max(1)
                    };

                    // Lay every round out as a contiguous, non-overlapping band of
                    // competition columns, in chronological order, with all finals
                    // forming the final band. Bands are sized in proportion to their
                    // game count so the schedule has a uniform density throughout
                    // instead of sparse early slots and a packed tail; each band is also
                    // kept at or above its minimum width so no division overflows. The
                    // finals band additionally keeps enough depth (one column per
                    // distinct stage, plus slack) for the QF -> SF -> GF / 3rd-place
                    // ordering, and absorbs any leftover columns as the last band.
                    let mut bands: Vec<(Vec<usize>, usize, usize)> = Vec::new(); // (round idxs, game count, min cols)
                    for &r_idx in &rr_rounds {
                        let count = internal_activities.iter().filter(|a| a.round_index == r_idx && !a.is_interview).count();
                        bands.push((vec![r_idx], count, min_cols(&|ri| ri == r_idx)));
                    }
                    if !finals_rounds.is_empty() {
                        let count = internal_activities.iter().filter(|a| a.round_index >= 10000 && !a.is_interview).count();
                        let depth = (finals_rounds.len() + 2).max(min_cols(&|ri| ri >= 10000));
                        bands.push((finals_rounds.clone(), count, depth));
                    }

                    let total_count: usize = bands.iter().map(|(_, c, _)| *c).sum::<usize>().max(1);
                    let n_bands = bands.len();
                    
                    let mut band_starts = vec![0; n_bands];
                    let mut band_ends = vec![0; n_bands];
                    let mut curr = 0usize;
                    for (bi, (_, count, min_c)) in bands.iter().enumerate() {
                        let remaining_min: usize = bands[bi + 1..].iter().map(|(_, _, m)| *m).sum();
                        let share = ((*count as f64 / total_count as f64) * n_comp as f64).round() as usize;
                        let mut c_end = curr + share.max(*min_c);
                        c_end = c_end.min(n_comp.saturating_sub(remaining_min)).max(curr + *min_c);
                        if bi == n_bands - 1 {
                            c_end = n_comp;
                        }
                        c_end = c_end.min(n_comp);
                        band_starts[bi] = curr;
                        band_ends[bi] = c_end;
                        curr = c_end;
                    }
                    
                    let finals_start_slot = if !finals_rounds.is_empty() && n_bands > 0 {
                        let finals_c_start = band_starts[n_bands - 1];
                        comp_indices[finals_c_start.min(n_comp - 1)]
                    } else {
                        n_slots
                    };
                    
                    for (bi, (round_idxs, _, _)) in bands.iter().enumerate() {
                        let is_finals = !finals_rounds.is_empty() && bi == n_bands - 1;
                        let range = if is_finals {
                            let range_start = comp_indices[band_starts[bi].min(n_comp - 1)];
                            range_start..n_slots
                        } else {
                            let range_start = comp_indices[band_starts[bi].min(n_comp - 1)];
                            range_start..finals_start_slot
                        };
                        for &r_idx in round_idxs {
                            round_ranges[r_idx] = range.clone();
                        }
                    }
                }
            }
        }

        Self {
            activities: internal_activities,
            volunteers: internal_volunteers,
            fields: internal_fields,
            slots: internal_slots,
            divisions: internal_divisions,
            teams: internal_teams,
            organizations,
            days: day_names,
            day_interviews_enabled,
            activity_buckets,
            activity_overlapping_slots,
            num_total_buckets,
            round_ranges,
            strict_capabilities: config.strict_capabilities,
            can_interview,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        DayGenConfig, Division, Field, FieldKind, FinalsRounds, SchedulingMode, Team, TimeSlot,
        TournamentConfig,
    };

    fn slot(id: &str, day: &str, start_min: u32, kind: FieldKind) -> TimeSlot {
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        TimeSlot {
            id: id.into(),
            day: day.into(),
            start_time: fmt(start_min),
            end_time: fmt(start_min + 15),
            kind,
        }
    }

    fn two_day_config() -> TournamentConfig {
        let mut config = TournamentConfig::default();
        config.divisions.push(Division {
            id: "d1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 3, volunteers_required: 0, duration_minutes: 15,
            allowed_fields: None, interviews_enabled: true, interview_volunteers_required: 0,
            interview_duration_minutes: 8, finals_enabled: true,
            finals_rounds: Some(FinalsRounds::Grand), finals_duration_minutes: None,
            finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        });
        for (t, org) in [("A", "o1"), ("B", "o1"), ("C", "o2"), ("D", "o2"), ("E", "o3"), ("F", "o3")] {
            config.teams.push(Team { name: t.into(), division_id: "d1".into(), organization: org.into() });
        }
        config.fields.push(Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None });
        config.fields.push(Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None });
        config.fields.push(Field { id: "it".into(), name: "Interview".into(), kind: FieldKind::Interview, allowed_divisions: None });

        // Day 1 carries competition slots AND interview slots (the interview slots
        // used to inflate day 1's share of the index space and pull all RR onto it).
        // Day 2 has competition slots only.
        for i in 0..5 {
            config.time_slots.push(slot(&format!("sat_c{i}"), "Saturday", 10 * 60 + i * 20, FieldKind::Competition));
            config.time_slots.push(slot(&format!("sat_i{i}"), "Saturday", 10 * 60 + i * 20 + 5, FieldKind::Interview));
        }
        for i in 0..4 {
            config.time_slots.push(slot(&format!("sun_c{i}"), "Sunday", 10 * 60 + i * 20, FieldKind::Competition));
        }
        config.day_configs.push(DayGenConfig { day: "Saturday".into(), ..Default::default() });
        config.day_configs.push(DayGenConfig { day: "Sunday".into(), interviews_enabled: false, ..Default::default() });
        config
    }

    #[test]
    fn rr_windows_reach_later_days_and_reserve_a_finals_tail() {
        let config = two_day_config();
        let activities = crate::scheduler::generate_activities(&config);
        let internal = InternalTournamentConfig::compile(&config, &activities);

        // Competition slot indices on the second day.
        let day2_comp: Vec<usize> = internal.slots.iter().enumerate()
            .filter(|(_, s)| s.day_idx == 1 && s.kind == FieldKind::Competition)
            .map(|(i, _)| i)
            .collect();
        assert!(!day2_comp.is_empty(), "fixture should have day-2 competition slots");
        let first_day2 = *day2_comp.iter().min().unwrap();
        let last_day2 = *day2_comp.iter().max().unwrap();

        // Round-robin rounds (round_index < 10000) must, collectively, be allowed to
        // use at least one day-2 competition slot — i.e. RR is no longer trapped on
        // day 1.
        let rr_reaches_day2 = internal.activities.iter()
            .filter(|a| !a.is_interview && a.round_index < 10000)
            .any(|a| {
                let r = &internal.round_ranges[a.round_index];
                day2_comp.iter().any(|&i| r.contains(&i))
            });
        assert!(rr_reaches_day2, "round-robin windows should extend into day 2");

        // Finals must sit at the tail: their window includes the very last day-2
        // competition slot, and starts strictly after the earliest RR start (so the
        // round-robin is not entirely buried inside the finals window).
        let finals = internal.activities.iter().find(|a| a.is_final).expect("a finals match");
        let finals_range = &internal.round_ranges[finals.round_index];
        assert!(finals_range.contains(&last_day2), "finals window should reach the last slot");

        let earliest_rr_start = internal.activities.iter()
            .filter(|a| !a.is_interview && a.round_index < 10000)
            .map(|a| internal.round_ranges[a.round_index].start)
            .min()
            .unwrap();
        assert!(finals_range.start > earliest_rr_start, "finals should start after RR begins");
        // The finals tail must not swallow the first day-2 competition slot, leaving
        // room for RR there.
        assert!(finals_range.start >= first_day2, "finals tail should be near the end, not mid-day-1");
    }
}
