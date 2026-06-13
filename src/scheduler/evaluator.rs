use crate::model::{
    Activity, FieldKind, Schedule, TournamentConfig,
};
use std::collections::HashMap;
use super::utils::{is_field_suitable_for_activity, is_volunteer_qualified, format_minutes_to_time};
use super::SolverParams;

type DivAssignInfoWithIdx = (usize, usize, usize, usize, bool, usize);
type DivAssignInfoWithName = (usize, usize, usize, usize, bool, String);

pub fn get_occupied_slots(config: &TournamentConfig, start_slot_id: &str, duration_minutes: u32) -> Vec<String> {
    let mut occupied = Vec::new();
    if let Some(start_slot) = config.time_slots.iter().find(|s| s.id == start_slot_id) {
        let start_min = start_slot.start_minutes();
        let end_min = start_min + duration_minutes;
        
        for slot in &config.time_slots {
            if slot.day.to_lowercase() == start_slot.day.to_lowercase() {
                let slot_start = slot.start_minutes();
                let slot_end = slot_start + slot.duration_minutes();
                
                if start_min < slot_end && slot_start < end_min {
                    occupied.push(slot.id.clone());
                }
            }
        }
    } else {
        occupied.push(start_slot_id.to_string());
    }
    occupied
}

/// Scores a schedule. Delegates to the solver's evaluator
/// ([`super::fast_evaluator::evaluate_schedule_cost`]) so cost display and
/// optimization share one implementation.
pub fn evaluate_schedule_cost(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> (f64, f64) {
    super::fast_evaluator::evaluate_schedule_cost(config, schedule, params)
}


/// Returns the indices of assignments that are currently involved in at least one hard conflict.
/// Used to bias mutation toward fixing broken assignments rather than touching healthy ones.
#[allow(dead_code)]
pub fn find_conflicted_assignment_indices(config: &TournamentConfig, schedule: &Schedule) -> Vec<usize> {
    let mut conflicted = vec![false; schedule.assignments.len()];

    // Build overlap maps: (bucket_idx, key) -> list of assignment indices
    let mut team_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    let mut field_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    let mut vol_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();

    for (i, assign) in schedule.assignments.iter().enumerate() {
        let slot_id = &assign.time_slot_id;
        let activity = &assign.activity;
        let div_id = activity.division_id();

        let division = match config.divisions.iter().find(|d| d.id == div_id) {
            Some(d) => d,
            None => { conflicted[i] = true; continue; }
        };

        let start_min = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| s.start_minutes()).unwrap_or(0);
        let end_min = start_min + activity.duration_minutes();
        let day_idx = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| {
            config.day_configs.iter().position(|dc| dc.day.to_lowercase() == s.day.to_lowercase()).unwrap_or(0)
        }).unwrap_or(0);
        let bucket_size = 5u32;
        let buckets_per_day = 24 * 60 / bucket_size;
        let day_offset = day_idx as u32 * buckets_per_day;

        let first_bucket = (start_min / bucket_size) as usize;
        let last_bucket = ((end_min - 1) / bucket_size) as usize;
        let buckets: Vec<usize> = (first_bucket..=last_bucket).map(|b| day_offset as usize + b).collect();

        for b_idx in &buckets {
            for team in activity.teams() {
                team_slot.entry((*b_idx, team.to_string())).or_default().push(i);
            }
        }

        if let Some(f_id) = &assign.field_id {
            for b_idx in &buckets {
                field_slot.entry((*b_idx, f_id.clone())).or_default().push(i);
            }
            let suitable = config.fields.iter().find(|f| f.id == *f_id)
                .is_some_and(|f| is_field_suitable_for_activity(config, f, activity));
            if !suitable { conflicted[i] = true; }
        }

        let req = match activity {
            Activity::Interview { .. } => division.interview_volunteers_required,
            _ => division.volunteers_required,
        };
        if assign.volunteer_ids.len() < req { conflicted[i] = true; }

        for vol_id in &assign.volunteer_ids {
            for b_idx in &buckets {
                vol_slot.entry((*b_idx, vol_id.clone())).or_default().push(i);
            }
            if let Some(vol) = config.volunteers.iter().find(|v| v.id == *vol_id) {
                // Attendance
                let slot = config.time_slots.iter().find(|s| s.id == *slot_id).unwrap();
                if matches!(vol.status_for_day(&slot.day), crate::model::AttendanceStatus::NoShow) {
                    conflicted[i] = true;
                }

                // Availability
                let duration = activity.duration_minutes();
                let day = config.time_slots.iter().find(|s| s.id == *slot_id).unwrap().day.to_lowercase();
                
                for slot in &config.time_slots {
                    if slot.day.to_lowercase() == day {
                        let s_start = slot.start_minutes();
                        let s_end = s_start + slot.duration_minutes();
                        if start_min < s_end && s_start < start_min + duration
                            && !vol.availabilities.contains(&slot.id) {
                                conflicted[i] = true;
                                break;
                            }
                    }
                }
                
                // Capability
                if !is_volunteer_qualified(vol, activity, div_id) {
                    let is_int = matches!(activity, Activity::Interview { .. });
                    let is_ivr = vol.capabilities.as_ref()
                        .is_some_and(|c| c.contains(&"Interview".to_string()));
                    if config.strict_capabilities || is_int || is_ivr { conflicted[i] = true; }
                }
                
                // Conflict of interest
                for team_name in activity.teams() {
                    if let Some(team) = config.teams.iter().find(|t| t.name == team_name)
                        && vol.conflict_organizations.contains(&team.organization) {
                            conflicted[i] = true;
                        }
                }
            } else {
                conflicted[i] = true;
            }
        }

        // Duration check: must fit within day
        let slot_ok = config.time_slots.iter().find(|s| s.id == *slot_id)
            .is_some_and(|s| {
                let day_end = config.time_slots.iter()
                    .filter(|other| other.day.to_lowercase() == s.day.to_lowercase())
                    .map(|other| other.start_minutes() + other.duration_minutes())
                    .max()
                    .unwrap_or(0);
                s.start_minutes() + activity.duration_minutes() <= day_end
            });
        if !slot_ok { conflicted[i] = true; }
    }

    for idxs in team_slot.values()  { if idxs.len() > 1 { for &i in idxs { conflicted[i] = true; } } }
    for idxs in field_slot.values() { if idxs.len() > 1 { for &i in idxs { conflicted[i] = true; } } }
    for idxs in vol_slot.values()   { if idxs.len() > 1 { for &i in idxs { conflicted[i] = true; } } }

    conflicted.iter().enumerate()
        .filter_map(|(i, &c)| if c { Some(i) } else { None })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConflictSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssignmentConflict {
    pub severity: ConflictSeverity,
    pub message: String,
}

pub fn get_assignment_conflicts(config: &TournamentConfig, schedule: &Schedule) -> HashMap<usize, Vec<AssignmentConflict>> {
    let mut result: HashMap<usize, Vec<AssignmentConflict>> = HashMap::new();

    // Build overlap maps: (bucket_idx, key) -> list of assignment indices
    let mut team_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    let mut field_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    let mut vol_slot: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    let mut div_assignments: HashMap<String, Vec<DivAssignInfoWithIdx>> = HashMap::new(); // div_id -> vec<(min_idx, max_idx, round_index, stage, is_int, assign_idx)>

    let slot_map: HashMap<&str, &crate::model::TimeSlot> = config
        .time_slots
        .iter()
        .map(|slot| (slot.id.as_str(), slot))
        .collect();

    let slot_idx_map: HashMap<&str, usize> = config
        .time_slots
        .iter()
        .enumerate()
        .map(|(idx, slot)| (slot.id.as_str(), idx))
        .collect();

    for (i, assign) in schedule.assignments.iter().enumerate() {
        let slot_id = &assign.time_slot_id;
        let activity = &assign.activity;
        let div_id = activity.division_id();

        let division = match config.divisions.iter().find(|d| d.id == div_id) {
            Some(d) => d,
            None => {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Internal Error: Division '{}' not found.", div_id),
                });
                continue;
            }
        };

        let activity_name = {
            let label = activity.label();
            // Remove emoji icons like ⚽, 🏆, 🤖, 💬 at the start for text logs
            if label.starts_with('⚽') || label.starts_with('🏆') || label.starts_with('🤖') || label.starts_with('💬') {
                label.chars().skip(2).collect::<String>()
            } else {
                label
            }
        };

        let occupied_slots = get_occupied_slots(config, slot_id, activity.duration_minutes());

        let mut min_idx = usize::MAX;
        let mut max_idx = 0;
        let mut has_idx = false;

        for slot_overlap_id in &occupied_slots {
            if let Some(&idx) = slot_idx_map.get(slot_overlap_id.as_str()) {
                min_idx = min_idx.min(idx);
                max_idx = max_idx.max(idx);
                has_idx = true;
            }
        }

        if has_idx {
            div_assignments.entry(div_id.to_string()).or_default().push((min_idx, max_idx, activity.round_index(), activity.stage(), matches!(activity, Activity::Interview { .. }), i));
        }

        let start_min = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| s.start_minutes()).unwrap_or(0);
        let end_min = start_min + activity.duration_minutes();
        let day_idx = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| {
            config.day_configs.iter().position(|dc| dc.day.to_lowercase() == s.day.to_lowercase()).unwrap_or(0)
        }).unwrap_or(0);
        let bucket_size = 5u32;
        let buckets_per_day = 24 * 60 / bucket_size;
        let day_offset = day_idx as u32 * buckets_per_day;

        let first_bucket = (start_min / bucket_size) as usize;
        let last_bucket = ((end_min - 1) / bucket_size) as usize;
        let buckets: Vec<usize> = (first_bucket..=last_bucket).map(|b| day_offset as usize + b).collect();

        for b_idx in &buckets {
            for team in activity.teams() {
                team_slot.entry((*b_idx, team.to_string())).or_default().push(i);
            }
        }

        if let Some(f_id) = &assign.field_id {
            for b_idx in &buckets {
                field_slot.entry((*b_idx, f_id.clone())).or_default().push(i);
            }
            let suitable = config.fields.iter().find(|f| f.id == *f_id)
                .is_some_and(|f| is_field_suitable_for_activity(config, f, activity));
            if !suitable {
                let field_name = config.fields.iter().find(|f| f.id == *f_id).map_or(f_id.clone(), |f| f.name.clone());
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Field Suitability: Field '{}' is not suitable for '{}'.", field_name, activity_name),
                });
            }
        } else {
            result.entry(i).or_default().push(AssignmentConflict {
                severity: ConflictSeverity::Error,
                message: format!("Field Missing: No field/arena assigned for '{}'.", activity_name),
            });
        }

        let req = match activity {
            Activity::Interview { .. } => division.interview_volunteers_required,
            _ => division.volunteers_required,
        };
        if assign.volunteer_ids.len() < req {
            result.entry(i).or_default().push(AssignmentConflict {
                severity: ConflictSeverity::Warning,
                message: format!("Under-Rostered: Requires at least {} volunteers, but only {} assigned.", req, assign.volunteer_ids.len()),
            });
        }

        for vol_id in &assign.volunteer_ids {
            for b_idx in &buckets {
                vol_slot.entry((*b_idx, vol_id.clone())).or_default().push(i);
            }
            if let Some(vol) = config.volunteers.iter().find(|v| v.id == *vol_id) {
                // Attendance
                let slot = config.time_slots.iter().find(|s| s.id == *slot_id).unwrap();
                if matches!(vol.status_for_day(&slot.day), crate::model::AttendanceStatus::NoShow) {
                    result.entry(i).or_default().push(AssignmentConflict {
                        severity: ConflictSeverity::Error,
                        message: format!("Volunteer Attendance: '{}' is marked as a NO-SHOW for {}.", vol.name, slot.day),
                    });
                }

                // Availability
                for slot_overlap_id in &occupied_slots {
                    if !vol.availabilities.contains(slot_overlap_id) {
                        let overlap_slot = slot_map.get(slot_overlap_id.as_str());
                        let overlap_slot_name = overlap_slot.map_or(slot_overlap_id.clone(), |s| format!("{} {}-{}", s.day, s.start_time, s.end_time));
                        result.entry(i).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: format!("Volunteer Availability: '{}' is not available during slot '{}'.", vol.name, overlap_slot_name),
                        });
                    }
                }
                
                // Capability
                if !is_volunteer_qualified(vol, activity, div_id) {
                    let is_int = matches!(activity, Activity::Interview { .. });
                    let is_ivr = vol.capabilities.as_ref()
                        .is_some_and(|c| c.contains(&"Interview".to_string()));
                    if config.strict_capabilities || is_int || is_ivr {
                        result.entry(i).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: format!("Volunteer Capability: '{}' lacks required qualifications for this activity.", vol.name),
                        });
                    }
                }
                
                // Conflict of interest
                for team_name in activity.teams() {
                    if let Some(team) = config.teams.iter().find(|t| t.name == team_name)
                        && vol.conflict_organizations.contains(&team.organization) {
                            result.entry(i).or_default().push(AssignmentConflict {
                                severity: ConflictSeverity::Error,
                                message: format!("Conflict of Interest: '{}' has a conflict with organization '{}' (Team '{}').", vol.name, team.organization, team_name),
                            });
                        }
                }
            } else {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Invalid Assignment: Unknown volunteer ID '{}'.", vol_id),
                });
            }
        }

        // Slot kind and Duration check
        if let Some(s) = slot_map.get(slot_id.as_str()) {
            let is_interview = matches!(activity, Activity::Interview { .. });
            if is_interview && s.kind == FieldKind::Competition {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: "Interview assigned to Competition time slot.".to_string(),
                });
            } else if !is_interview && s.kind == FieldKind::Interview {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: "Match/Run assigned to Interview time slot.".to_string(),
                });
            }

            if is_interview {
                let day_cfg = config.day_configs.iter().find(|dc| dc.day.to_lowercase() == s.day.to_lowercase());
                if let Some(dc) = day_cfg
                    && !dc.interviews_enabled {
                        result.entry(i).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: format!("Interviews are disabled on {}.", dc.day),
                        });
                    }
            }

            let day_end = config.time_slots.iter()
                .filter(|other| other.day.to_lowercase() == s.day.to_lowercase())
                .map(|other| other.start_minutes() + other.duration_minutes())
                .max()
                .unwrap_or(0);
            if s.start_minutes() + activity.duration_minutes() > day_end {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Duration Error: Activity ({} min) exceeds the end of the day.", activity.duration_minutes()),
                });
            }
        }
    }

    // Process overlaps
    for ((bucket_idx, team), idxs) in team_slot {
        if idxs.len() > 1 {
            let bucket_size = 5u32;
            let buckets_per_day = 24 * 60 / bucket_size;
            let day_idx = bucket_idx / buckets_per_day as usize;
            let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
            let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
            let time_str = format_minutes_to_time(start_min as u32);
            let slot_name = format!("{} {}", day_name, time_str);

            for &i in &idxs {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Team Double-Booking: Team '{}' is scheduled for {} activities simultaneously during slot '{}'.", team, idxs.len(), slot_name),
                });
            }
        }
    }
    for ((bucket_idx, f_id), idxs) in field_slot {
        if idxs.len() > 1 {
            let bucket_size = 5u32;
            let buckets_per_day = 24 * 60 / bucket_size;
            let day_idx = bucket_idx / buckets_per_day as usize;
            let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
            let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
            let time_str = format_minutes_to_time(start_min as u32);
            let slot_name = format!("{} {}", day_name, time_str);

            let field_name = config.fields.iter().find(|f| f.id == f_id).map_or(f_id, |f| f.name.clone());
            for &i in &idxs {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Error,
                    message: format!("Field Double-Booking: Field/Arena '{}' is double-booked during slot '{}'.", field_name, slot_name),
                });
            }
        }
    }
    // Process volunteer double-booking
    for ((bucket_idx, vol_id), idxs) in vol_slot {
        if idxs.len() > 1 {
            let bucket_size = 5u32;
            let buckets_per_day = 24 * 60 / bucket_size;
            let day_idx = bucket_idx / buckets_per_day as usize;
            let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
            let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
            let time_str = format_minutes_to_time(start_min as u32);
            let slot_name = format!("{} {}", day_name, time_str);

            let vol_name = config.volunteers.iter().find(|v| v.id == vol_id).map_or(vol_id, |v| v.name.clone());
            for &i in &idxs {
                result.entry(i).or_default().push(AssignmentConflict {
                    severity: ConflictSeverity::Warning, // User asked for volunteer conflict as warning
                    message: format!("Volunteer Double-Booking: Volunteer '{}' is double-booked during slot '{}'.", vol_name, slot_name),
                });
            }
        }
    }

    // Process RR-Finals overlaps and stage ordering
    for assignments in div_assignments.values_mut() {
        assignments.sort_by_key(|a| a.0);
        for i in 0..assignments.len() {
            for j in i + 1..assignments.len() {
                let (_min1, max1, _round1, stage1, is_int1, idx1) = assignments[i];
                let (min2, _max2, _round2, stage2, is_int2, idx2) = assignments[j];

                if is_int1 || is_int2 { continue; }

                // Same division, chronological order: _min1 <= min2
                if stage1 != stage2 {
                    let is_3pl_gf = (stage1 == 4 && stage2 == 5) || (stage1 == 5 && stage2 == 4);
                    if is_3pl_gf {
                        // Exception: 3PL and GF can overlap or happen in any order.
                        continue;
                    }
                    
                    if stage1 > stage2 {
                        let msg = "Stage Order: Matches from a later stage (e.g. Finals) are scheduled before matches from an earlier stage (e.g. Round Robin).".to_string();
                        result.entry(idx1).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: msg.clone(),
                        });
                        result.entry(idx2).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: msg,
                        });
                    } else if max1 >= min2 {
                        // stage1 < stage2: check for temporal overlap
                        let msg = "Stage Overlap: Matches from an earlier stage (e.g. Round Robin) should not happen during a later stage (e.g. Finals).".to_string();
                        result.entry(idx1).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: msg.clone(),
                        });
                        result.entry(idx2).or_default().push(AssignmentConflict {
                            severity: ConflictSeverity::Error,
                            message: msg,
                        });
                    }
                }
            }
        }
    }

    result
}

pub fn get_schedule_conflicts(config: &TournamentConfig, schedule: &Schedule) -> Vec<String> {
    let mut conflicts = Vec::new();

    let mut team_overlap = HashMap::new();
    let mut field_overlap = HashMap::new();
    let mut volunteer_overlap = HashMap::new();

    let mut division_assignments: HashMap<String, Vec<DivAssignInfoWithName>> = HashMap::new(); // div_id -> vec<(min_idx, max_idx, round_index, stage, is_int, activity_name)>

    let slot_map: HashMap<&str, &crate::model::TimeSlot> = config
        .time_slots
        .iter()
        .map(|slot| (slot.id.as_str(), slot))
        .collect();

    let slot_idx_map: HashMap<&str, usize> = config
        .time_slots
        .iter()
        .enumerate()
        .map(|(idx, slot)| (slot.id.as_str(), idx))
        .collect();

    for assign in &schedule.assignments {
        let slot_id = &assign.time_slot_id;
        let field_id = &assign.field_id;
        let activity = &assign.activity;
        let div_id = activity.division_id();
        let division = match config.divisions.iter().find(|d| d.id == div_id) {
            Some(d) => d,
            None => {
                conflicts.push(format!("Internal Error: Division '{}' not found in config.", div_id));
                continue;
            }
        };

        let slot_name = slot_map.get(slot_id.as_str())
            .map_or(slot_id.clone(), |s| format!("{} {}-{}", s.day, s.start_time, s.end_time));

        let activity_name = {
            let label = activity.label();
            // Remove emoji icons like ⚽, 🏆, 🤖, 💬 at the start for text logs
            if label.starts_with('⚽') || label.starts_with('🏆') || label.starts_with('🤖') || label.starts_with('💬') {
                label.chars().skip(2).collect::<String>()
            } else {
                label
            }
        };

        let occupied_slots = get_occupied_slots(config, slot_id, activity.duration_minutes());

        let mut min_idx = usize::MAX;
        let mut max_idx = 0;
        let mut has_idx = false;

        for slot_overlap_id in &occupied_slots {
            if let Some(&idx) = slot_idx_map.get(slot_overlap_id.as_str()) {
                min_idx = min_idx.min(idx);
                max_idx = max_idx.max(idx);
                has_idx = true;
            }
        }

        if has_idx {
            division_assignments.entry(div_id.to_string()).or_default().push((min_idx, max_idx, activity.round_index(), activity.stage(), matches!(activity, Activity::Interview { .. }), activity_name.clone()));
        }

        let start_min = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| s.start_minutes()).unwrap_or(0);
        let end_min = start_min + activity.duration_minutes();
        let day_idx = config.time_slots.iter().find(|s| s.id == *slot_id).map(|s| {
            config.day_configs.iter().position(|dc| dc.day.to_lowercase() == s.day.to_lowercase()).unwrap_or(0)
        }).unwrap_or(0);
        let bucket_size = 5u32;
        let buckets_per_day = 24 * 60 / bucket_size;
        let day_offset = day_idx as u32 * buckets_per_day;

        let first_bucket = (start_min / bucket_size) as usize;
        let last_bucket = ((end_min - 1) / bucket_size) as usize;
        let buckets: Vec<usize> = (first_bucket..=last_bucket).map(|b| day_offset as usize + b).collect();

        for b_idx in &buckets {
            for team in activity.teams() {
                let entry = team_overlap
                    .entry(*b_idx)
                    .or_insert_with(HashMap::new);
                *entry.entry(team.to_string()).or_insert(0) += 1;
            }
        }

        if let Some(f_id) = field_id {
            let field = config.fields.iter().find(|f| f.id == *f_id);
            let field_name = field.map_or(f_id.clone(), |f| f.name.clone());

            for b_idx in &buckets {
                let entry = field_overlap
                    .entry(*b_idx)
                    .or_insert_with(HashMap::new);
                *entry.entry(f_id.clone()).or_insert(0) += 1;
            }

            let field_suitable = field.is_some_and(|f| {
                is_field_suitable_for_activity(config, f, activity)
            });
            if !field_suitable {
                conflicts.push(format!(
                    "Field Suitability: Field '{}' is not suitable for '{}' in slot '{}'.",
                    field_name, activity_name, slot_name
                ));
            }
        } else {
            conflicts.push(format!(
                "Field Missing: No field/arena assigned for '{}' in slot '{}'.",
                activity_name, slot_name
            ));
        }

        for volunteer_id in &assign.volunteer_ids {
            for b_idx in &buckets {
                let entry = volunteer_overlap
                    .entry(*b_idx)
                    .or_insert_with(HashMap::new);
                *entry.entry(volunteer_id.clone()).or_insert(0) += 1;
            }

            if let Some(volunteer) = config.volunteers.iter().find(|v| v.id == *volunteer_id) {
                for slot_overlap_id in &occupied_slots {
                    if !volunteer.availabilities.contains(slot_overlap_id) {
                        let overlap_slot = slot_map.get(slot_overlap_id.as_str());
                        let overlap_slot_name = overlap_slot.map_or(slot_overlap_id.clone(), |s| format!("{} {}-{}", s.day, s.start_time, s.end_time));
                        conflicts.push(format!(
                            "Volunteer Availability: Volunteer '{}' is assigned to '{}' which spans to slot '{}' but is not marked as available then.",
                            volunteer.name, activity_name, overlap_slot_name
                        ));
                    }
                }

                if !is_volunteer_qualified(volunteer, activity, div_id) {
                    let is_interview_activity = matches!(activity, Activity::Interview { .. });
                    let is_interviewer = volunteer.capabilities.as_ref().is_some_and(|caps| caps.contains(&"Interview".to_string()));

                    if config.strict_capabilities || is_interview_activity || is_interviewer {
                        let required_cap_desc = if is_interview_activity {
                            "Interview capability".to_string()
                        } else {
                            format!("qualification for division '{}'", division.name)
                        };
                        conflicts.push(format!(
                            "Volunteer Capability: Volunteer '{}' is assigned to '{}' in slot '{}' but lacks required {}.",
                            volunteer.name, activity_name, slot_name, required_cap_desc
                        ));
                    }
                }

                for team_name in activity.teams() {
                    if let Some(team) = config.teams.iter().find(|t| t.name == team_name)
                        && volunteer.conflict_organizations.contains(&team.organization) {
                            conflicts.push(format!(
                                "Conflict of Interest: Volunteer '{}' has a conflict of interest with organization '{}' (Team '{}') scheduled in slot '{}'.",
                                volunteer.name, team.organization, team_name, slot_name
                            ));
                        }
                }
            } else {
                conflicts.push(format!(
                    "Invalid Assignment: Unknown volunteer ID '{}' is assigned in slot '{}'.",
                    volunteer_id, slot_name
                ));
            }
        }

        let req_volunteers = match activity {
            Activity::Interview { .. } => division.interview_volunteers_required,
            _ => division.volunteers_required,
        };
        if assign.volunteer_ids.len() < req_volunteers {
            conflicts.push(format!(
                "Under-Rostered: '{}' in slot '{}' has only {} volunteer(s) assigned, but requires at least {}.",
                activity_name, slot_name, assign.volunteer_ids.len(), req_volunteers
            ));
        }

        if let Some(slot) = slot_map.get(slot_id.as_str()) {
            let is_interview = matches!(activity, Activity::Interview { .. });
            if is_interview && slot.kind == FieldKind::Competition {
                conflicts.push(format!("Slot Type Error: Interview '{}' assigned to Competition time slot '{}'.", activity_name, slot_name));
            } else if !is_interview && slot.kind == FieldKind::Interview {
                conflicts.push(format!("Slot Type Error: Match/Run '{}' assigned to Interview time slot '{}'.", activity_name, slot_name));
            }

            if is_interview {
                let day_cfg = config.day_configs.iter().find(|dc| dc.day.to_lowercase() == slot.day.to_lowercase());
                if let Some(dc) = day_cfg
                    && !dc.interviews_enabled {
                        conflicts.push(format!("Interviews are disabled on {}.", dc.day));
                    }
            }

            let day_end = config.time_slots.iter()
                .filter(|s| s.day.to_lowercase() == slot.day.to_lowercase())
                .map(|s| s.start_minutes() + s.duration_minutes())
                .max()
                .unwrap_or(0);
            if slot.start_minutes() + activity.duration_minutes() > day_end {
                conflicts.push(format!(
                    "Duration Error: Activity '{}' ({} min) starting at slot '{}' exceeds the end of the day.",
                    activity_name, activity.duration_minutes(), slot_name
                ));
            }
        }
    }

    for (bucket_idx, teams) in team_overlap {
        for (team, count) in teams {
            if count > 1 {
                let bucket_size = 5u32;
                let buckets_per_day = 24 * 60 / bucket_size;
                let day_idx = bucket_idx / buckets_per_day as usize;
                let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
                let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
                let time_str = format_minutes_to_time(start_min as u32);
                conflicts.push(format!(
                    "Team Double-Booking: Team '{}' is scheduled for {} activities simultaneously around {} {}.",
                    team, count, day_name, time_str
                ));
            }
        }
    }

    for (bucket_idx, fields) in field_overlap {
        for (f_id, count) in fields {
            if count > 1 {
                let bucket_size = 5u32;
                let buckets_per_day = 24 * 60 / bucket_size;
                let day_idx = bucket_idx / buckets_per_day as usize;
                let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
                let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
                let time_str = format_minutes_to_time(start_min as u32);
                let field_name = config.fields.iter().find(|f| f.id == f_id).map_or(f_id, |f| f.name.clone());
                conflicts.push(format!(
                    "Field Double-Booking: Field/Arena '{}' is double-booked for {} activities around {} {}.",
                    field_name, count, day_name, time_str
                ));
            }
        }
    }

    for (bucket_idx, volunteers) in volunteer_overlap {
        for (vol_id, count) in volunteers {
            if count > 1 {
                let bucket_size = 5u32;
                let buckets_per_day = 24 * 60 / bucket_size;
                let day_idx = bucket_idx / buckets_per_day as usize;
                let start_min = (bucket_idx % buckets_per_day as usize) * bucket_size as usize;
                let day_name = config.day_configs.get(day_idx).map(|dc| dc.day.as_str()).unwrap_or("Unknown");
                let time_str = format_minutes_to_time(start_min as u32);
                let vol_name = config.volunteers.iter().find(|v| v.id == vol_id).map_or(vol_id, |v| v.name.clone());
                conflicts.push(format!(
                    "Volunteer Double-Booking: Volunteer '{}' is double-booked for {} duties around {} {}.",
                    vol_name, count, day_name, time_str
                ));
            }
        }
    }

    // Process RR-Finals overlaps and stage ordering
    for (div_id, assignments) in &mut division_assignments {
        assignments.sort_by_key(|a| a.0);
        let div_name = config.divisions.iter().find(|d| d.id == *div_id).map(|d| d.name.as_str()).unwrap_or(div_id.as_str());
        for i in 0..assignments.len() {
            for j in i + 1..assignments.len() {
                let (min1, max1, _round1, stage1, is_int1, name1) = &assignments[i];
                let (min2, _max2, _round2, stage2, is_int2, name2) = &assignments[j];

                if *is_int1 || *is_int2 { continue; }

                // Same division, chronological order: min1 <= min2
                if stage1 != stage2 {
                    let is_3pl_gf = (*stage1 == 4 && *stage2 == 5) || (*stage1 == 5 && *stage2 == 4);
                    if is_3pl_gf {
                        // Exception: 3PL and GF can overlap or happen in any order.
                        continue;
                    }

                    if stage1 > stage2 {
                        let day1 = &config.time_slots[*min1].day;
                        let time1 = &config.time_slots[*min1].start_time;
                        let day2 = &config.time_slots[*min2].day;
                        let time2 = &config.time_slots[*min2].start_time;
                        conflicts.push(format!(
                            "Stage Order: In division '{}', later stage match '{}' ({} {}) is scheduled before earlier stage match '{}' ({} {}).",
                            div_name, name1, day1, time1, name2, day2, time2
                        ));
                    } else if *max1 >= *min2 {
                        // stage1 < stage2: check for temporal overlap
                        let day = &config.time_slots[*min1].day;
                        let time1 = &config.time_slots[*min1].start_time;
                        let time2 = &config.time_slots[*min2].start_time;
                        conflicts.push(format!(
                            "Stage Overlap: In division '{}', match '{}' ({}) overlaps with later stage match '{}' ({}) on {}.",
                            div_name, name1, time1, name2, time2, day
                        ));
                    }
                }
            }
        }
    }

    conflicts.sort();
    conflicts
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::scheduler::SolverParams;

    #[test]
    fn test_field_balance_overlapping_pools() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div1".into()]) },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div2".into()]) },
            Field { id: "f3".into(), name: "Field 3".into(), kind: FieldKind::Competition, allowed_divisions: None }, // Unrestricted
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
            Division { 
                id: "div2".into(), name: "Div 2".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        let params = SolverParams::default();

        // Schedule 1: Field 3 is overloaded with matches
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: Matches are balanced across fields
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert!(cost1.1 > cost2.1);
    }

    #[test]
    fn test_field_balance_with_interviews() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div1".into()]) },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div2".into()]) },
            Field { id: "f3".into(), name: "Field 3".into(), kind: FieldKind::Interview, allowed_divisions: None },
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
            Division { 
                id: "div2".into(), name: "Div 2".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        let params = SolverParams::default();

        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert!(cost1.1 > cost2.1);
    }

    #[test]
    fn test_round_order_penalty() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        let mut params = SolverParams::default();
        params.round_order_weight = 10.0;

        // Schedule 1: Round 1 at 09:00, Round 2 at 09:30 (Correct Order)
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_2_c0_r1".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 1 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: Round 2 at 09:00, Round 1 at 09:30 (Inverted Order)
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_2_c0_r1".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 1 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert_eq!(cost1.1, 0.0);
        assert!(cost2.1 >= 10.0);
    }

    #[test]
    fn test_rr_finals_overlap_hard_conflict() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 2, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: true, finals_rounds: Some(FinalsRounds::Grand), finals_duration_minutes: Some(20),
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:20".into(), end_time: "09:40".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![
            DayGenConfig { day: "Sat".into(), ..Default::default() },
        ];

        let mut params = SolverParams::default();
        params.round_order_weight = 1000.0;

        // Schedule 1: RR and Finals overlap at the same time (s1)
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_gf".into(), team_a: "1st".into(), team_b: "2nd".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::GrandFinal }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: RR and Finals are sequential (RR at s1, Finals at s2)
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_gf".into(), team_a: "1st".into(), team_b: "2nd".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::GrandFinal }, 
                    time_slot_id: "s2".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        // Schedule 1 should have at least 1 hard conflict
        assert!(cost1.0 >= 1.0, "Overlap should be a hard conflict (got {})", cost1.0);
        // Schedule 2 should have 0 hard conflicts
        assert_eq!(cost2.0, 0.0, "Sequential RR and Finals should have no hard conflicts");
        
        // Also check conflicts report
        let conflicts1 = get_schedule_conflicts(&config, &schedule1);
        assert!(conflicts1.iter().any(|c| c.contains("Stage Overlap")));
    }

    #[test]
    fn test_sf_3pl_order_hard_conflict() {
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

        let mut params = SolverParams::default();
        params.round_order_weight = 1000.0;

        // Schedule: 3rd Place Playoff at s1 (09:00), Semi Final at s2 (09:30)
        // This is INVERTED order.
        let schedule = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_3pl".into(), team_a: "L1".into(), team_b: "L2".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::ThirdPlace }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_sf_1".into(), team_a: "1st".into(), team_b: "4th".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::SemiFinal }, 
                    time_slot_id: "s2".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost = evaluate_schedule_cost(&config, &schedule, &params);
        let conflicts = get_schedule_conflicts(&config, &schedule);

        assert!(cost.0 >= 10.0, "3PL before SF should be a hard conflict (got {})", cost.0);
        assert!(conflicts.iter().any(|c| c.contains("Stage Order")), "Conflict message should contain 'Stage Order'");
    }
}
