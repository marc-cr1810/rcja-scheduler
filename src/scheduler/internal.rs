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
            InternalVolunteer {
                id: v.id.clone(),
                availability_slots: avail,
                capability_indices: v.capabilities.as_ref().map(|caps| {
                    caps.iter().filter_map(|c| div_map.get(c).copied()).collect()
                }),
                conflict_org_indices: v.conflict_organizations.iter().filter_map(|o| org_map.get(o).copied()).collect(),
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

                // Proportional windowing for RR rounds
                let mut rr_counts = HashMap::new();
                for &r_idx in &rr_rounds {
                    rr_counts.insert(r_idx, internal_activities.iter().filter(|a| a.round_index == r_idx && !a.is_interview).count());
                }
                let total_rr_acts: usize = rr_counts.values().sum();
                
                let mut finals_count = 0;
                for &r_idx in &finals_rounds {
                    finals_count += internal_activities.iter().filter(|a| a.round_index == r_idx && !a.is_interview).count();
                }

                let total_acts = total_rr_acts + finals_count;
                
                // We'll give RR rounds a portion of the timeline, and Finals rounds the remainder.
                // However, we'll make ALL finals rounds share the SAME range at the end to allow for more flexibility
                // and avoid the "empty range" fallback if one stage is too tight.
                
                let mut current_start = 0;
                for &r_idx in &rr_rounds {
                    let count = *rr_counts.get(&r_idx).unwrap();
                    let share = if total_acts > 0 { (count as f64 / total_acts as f64) * n_slots as f64 } else { 0.0 };
                    let mut end = (current_start as f64 + share).round() as usize;
                    
                    let max_dur_slots = internal_activities.iter()
                        .filter(|a| a.round_index == r_idx && !a.is_interview)
                        .map(|a| {
                            let slot_dur = internal_slots[0].duration_minutes.max(1);
                            a.duration_minutes.div_ceil(slot_dur) as usize
                        }).max().unwrap_or(1);
                    
                    end = end.max(current_start + max_dur_slots).min(n_slots);
                    round_ranges[r_idx] = current_start..end;
                    current_start = end;
                }

                if !finals_rounds.is_empty() {
                    let finals_range = current_start..n_slots;
                    for &r_idx in &finals_rounds {
                        round_ranges[r_idx] = finals_range.clone();
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
