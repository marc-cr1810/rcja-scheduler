use crate::model::{TournamentConfig, Activity, FieldKind};
use crate::scheduler::{generate_activities, is_field_suitable_for_activity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct DiagnosticMessage {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub recommendation: Option<String>,
}

pub fn validate_config(config: &TournamentConfig) -> Vec<DiagnosticMessage> {
    let mut diagnostics = Vec::new();

    let activities = generate_activities(config);
    let slots = &config.time_slots;
    let fields = &config.fields;
    let volunteers = &config.volunteers;

    // 1. Basic config checks
    if config.competition_name.trim().is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Warning,
            message: "Competition name is empty.".to_string(),
            recommendation: Some("Set a competition name on the dashboard.".to_string()),
        });
    }

    if slots.is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Error,
            message: "No time slots defined. You must add at least one time slot.".to_string(),
            recommendation: Some("Create time slots under the 'Time Slots' tab.".to_string()),
        });
    }

    if fields.is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Error,
            message: "No fields or arenas defined. You must add at least one field.".to_string(),
            recommendation: Some("Create fields/arenas under the 'Fields & Arenas' tab.".to_string()),
        });
    }

    if config.divisions.is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Info,
            message: "No divisions defined yet.".to_string(),
            recommendation: Some("Add a division to begin scheduling.".to_string()),
        });
    }

    if config.teams.is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Info,
            message: "No teams added yet.".to_string(),
            recommendation: Some("Add teams under the 'Teams' tab.".to_string()),
        });
    }

    if config.volunteers.is_empty() {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Warning,
            message: "No volunteers added yet.".to_string(),
            recommendation: Some("Add volunteers under the 'Volunteers' tab.".to_string()),
        });
    }

    // Duplicate detection
    let mut div_names = std::collections::HashSet::new();
    for div in &config.divisions {
        if !div_names.insert(&div.name) {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!("Duplicate division name: '{}'.", div.name),
                recommendation: Some("Ensure each division has a unique name.".to_string()),
            });
        }
    }

    let mut team_names = std::collections::HashSet::new();
    for team in &config.teams {
        if !team_names.insert(&team.name) {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!("Duplicate team name: '{}'.", team.name),
                recommendation: Some("Ensure each team has a unique name.".to_string()),
            });
        }
        if !config.divisions.iter().any(|d| d.id == team.division_id) {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!("Team '{}' belongs to a non-existent division.", team.name),
                recommendation: Some("Re-assign the team to a valid division.".to_string()),
            });
        }
    }

    let mut field_names = std::collections::HashSet::new();
    for field in &config.fields {
        if !field_names.insert(&field.name) {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!("Duplicate field name: '{}'.", field.name),
                recommendation: Some("Ensure each field has a unique name.".to_string()),
            });
        }
    }

    let mut vol_names = std::collections::HashSet::new();
    for vol in &config.volunteers {
        if !vol_names.insert(&vol.name) {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Warning,
                message: format!("Duplicate volunteer name: '{}'.", vol.name),
                recommendation: Some("Ensure volunteers have unique names to avoid confusion.".to_string()),
            });
        }
        if vol.availabilities.is_empty() {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Warning,
                message: format!("Volunteer '{}' has no available time slots.", vol.name),
                recommendation: Some("Select at least one time slot where this volunteer is available.".to_string()),
            });
        }
    }

    if slots.is_empty() {
        return diagnostics; // cannot run other checks if no slots
    }

    // 2. Capacity Check per Division
    for div in &config.divisions {
        let div_teams: Vec<&crate::model::Team> = config.teams.iter()
            .filter(|t| t.division_id == div.id)
            .collect();
        let div_teams_count = div_teams.len();

        if div_teams_count == 0 {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Info,
                message: format!("Division '{}' has no teams assigned.", div.name),
                recommendation: Some("Add teams to this division under the 'Teams' tab.".to_string()),
            });
            continue;
        }

        let div_activities: Vec<&Activity> = activities
            .iter()
            .filter(|a| a.division_id() == div.id)
            .collect();
        
        let required_games = div_activities
            .iter()
            .filter(|a| matches!(a, Activity::Match { .. } | Activity::Run { .. }))
            .count();

        // Find fields allowed for this division
        let div_fields: Vec<&crate::model::Field> = fields
            .iter()
            .filter(|f| {
                let dummy_activity = if div.mode == crate::model::SchedulingMode::HeadToHead {
                    Activity::Match { id: "dummy".to_string(), team_a: "A".to_string(), team_b: "B".to_string(), division_id: div.id.clone(), duration_minutes: div.duration_minutes, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }
                } else {
                    Activity::Run { id: "dummy".to_string(), team: "A".to_string(), division_id: div.id.clone(), run_number: 1, duration_minutes: div.duration_minutes }
                };
                is_field_suitable_for_activity(config, f, &dummy_activity)
            })
            .collect();

        if div_fields.is_empty() && required_games > 0 {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Division '{}' has no compatible fields or arenas.",
                    div.name
                ),
                recommendation: Some(format!(
                    "Ensure there is a field of kind 'Competition' that allows '{}', and that '{}' allows that field.",
                    div.name, div.name
                )),
            });
            continue;
        }

        // Count how many time slots are long enough for the division's games
        let compatible_slots = slots
            .iter()
            .filter(|s| {
                let day_end = slots
                    .iter()
                    .filter(|other| other.day.to_lowercase() == s.day.to_lowercase())
                    .map(|other| other.start_minutes() + other.duration_minutes())
                    .max()
                    .unwrap_or(0);
                s.start_minutes() + div.duration_minutes <= day_end
            })
            .count();

        if compatible_slots == 0 && required_games > 0 {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Division '{}' games require {} minutes, but no time slots can accommodate this duration.",
                    div.name, div.duration_minutes
                ),
                recommendation: Some(format!(
                    "Increase the duration of one or more time slots to at least {} minutes, or ensure there is enough contiguous slot time at the end of the day.",
                    div.duration_minutes
                )),
            });
        }

        if div.interviews_enabled {
            let interview_fields: Vec<&crate::model::Field> = fields.iter().filter(|f| {
                let dummy_interview = Activity::Interview { id: "dummy".to_string(), team: "A".to_string(), division_id: div.id.clone(), duration_minutes: div.interview_duration_minutes };
                is_field_suitable_for_activity(config, f, &dummy_interview)
            }).collect();

            if interview_fields.is_empty() {
                diagnostics.push(DiagnosticMessage {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Interviews are enabled for '{}', but no compatible interview fields or arenas were found.",
                        div.name
                    ),
                    recommendation: Some(
                        format!("Set up at least one field of kind 'Interview' that is compatible with '{}'.", div.name)
                    ),
                });
            }

            let compatible_interview_slots = slots
                .iter()
                .filter(|s| {
                    let day_end = slots
                        .iter()
                        .filter(|other| other.day.to_lowercase() == s.day.to_lowercase())
                        .map(|other| other.start_minutes() + other.duration_minutes())
                        .max()
                        .unwrap_or(0);
                    s.start_minutes() + div.interview_duration_minutes <= day_end
                })
                .count();
            if compatible_interview_slots == 0 {
                diagnostics.push(DiagnosticMessage {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Interviews for '{}' require {} minutes, but no time slots can accommodate this duration.",
                        div.name, div.interview_duration_minutes
                    ),
                    recommendation: Some(format!(
                        "Increase the duration of one or more time slots to at least {} minutes, or ensure there is enough contiguous slot time at the end of the day.",
                        div.interview_duration_minutes
                    )),
                });
            }

            if !interview_fields.is_empty() && compatible_interview_slots > 0 {
                let required_interviews = config.teams.iter().filter(|t| t.division_id == div.id).count();
                let total_interview_slots_available = interview_fields.len() * compatible_interview_slots;
                if required_interviews > total_interview_slots_available {
                    let def = required_interviews - total_interview_slots_available;
                    let recommended_fields = required_interviews.div_ceil(compatible_interview_slots);
                    let recommended_slots = required_interviews.div_ceil(interview_fields.len());

                    diagnostics.push(DiagnosticMessage {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Division '{}' requires {} team interviews, but only {} interview table-slots are available ({} interview tables * {} time slots). Shortfall of {}.",
                            div.name, required_interviews, total_interview_slots_available, interview_fields.len(), compatible_interview_slots, def
                        ),
                        recommendation: Some(format!(
                            "Add at least {} more interview tables (to reach {}) OR add {} more compatible time slots (to reach {}).",
                            recommended_fields.saturating_sub(interview_fields.len()),
                            recommended_fields,
                            recommended_slots.saturating_sub(compatible_interview_slots),
                            recommended_slots
                        )),
                    });
                }
            }
        }

        let total_game_slots_available = div_fields.len() * compatible_slots;
        if required_games > total_game_slots_available {
            let def = required_games - total_game_slots_available;
            let recommended_fields = required_games.div_ceil(compatible_slots);
            let recommended_slots = required_games.div_ceil(div_fields.len());

            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Division '{}' requires {} games/runs, but only {} field-slots are available ({} fields * {} time slots). Shortfall of {}.",
                    div.name, required_games, total_game_slots_available, div_fields.len(), compatible_slots, def
                ),
                recommendation: Some(format!(
                    "Add at least {} more fields (to reach {}) OR add {} more compatible time slots (to reach {}).",
                    recommended_fields.saturating_sub(div_fields.len()),
                    recommended_fields,
                    recommended_slots.saturating_sub(compatible_slots),
                    recommended_slots
                )),
            });
        }

        // 3. Team Activity Count Check
        // Count maximum activities for any team in this division
        let div_teams: Vec<&crate::model::Team> = config.teams.iter()
            .filter(|t| t.division_id == div.id)
            .collect();
        let div_teams_count = div_teams.len();

        if div.mode == crate::model::SchedulingMode::HeadToHead && div_teams_count < 2 {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Division '{}' is set to Head-to-Head (Soccer) but has only {} teams.",
                    div.name, div_teams_count
                ),
                recommendation: Some(format!(
                    "Add at least 2 teams to '{}' to allow match generation.",
                    div.name
                )),
            });
        }

        if div.finals_enabled {
            let required_teams = match div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) {
                crate::model::FinalsRounds::Grand => 2,
                crate::model::FinalsRounds::Semis => 4,
                crate::model::FinalsRounds::Quarter => 8,
                crate::model::FinalsRounds::Eighths => 16,
            };

            if div_teams_count < required_teams {
                diagnostics.push(DiagnosticMessage {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Division '{}' has {} teams, but the finals format requires at least {} teams.",
                        div.name, div_teams_count, required_teams
                    ),
                    recommendation: Some(format!(
                        "Add more teams to '{}' or change the finals format to a smaller group (e.g. Grand Final).",
                        div.name
                    )),
                });
            }
        }

        let finals_count = if div.finals_enabled {
            match div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) {
                crate::model::FinalsRounds::Grand => 1,
                crate::model::FinalsRounds::Semis => 2,
                crate::model::FinalsRounds::Quarter => 3,
                crate::model::FinalsRounds::Eighths => 4,
            }
        } else {
            0
        };
        let games_per_team = match div.mode {
            crate::model::SchedulingMode::HeadToHead => {
                if div_teams_count < 2 {
                    0
                } else {
                    div.games_per_team + finals_count
                }
            }
            crate::model::SchedulingMode::IndividualRun => div.games_per_team,
        };
        let mut activities_per_team = games_per_team;
        if div.interviews_enabled {
            activities_per_team += 1;
        }

        if activities_per_team > slots.len() {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Teams in division '{}' are scheduled for {} activities ({} games + {} interview), but there are only {} total time slots. A team cannot play in multiple slots simultaneously.",
                    div.name, activities_per_team, games_per_team, if div.interviews_enabled { 1 } else { 0 }, slots.len()
                ),
                recommendation: Some(format!(
                    "Increase the number of time slots to at least {} to accommodate all team activities.",
                    activities_per_team
                )),
            });
        }
    }

    // 4. Team Total Activity Check (Across the entire competition)
    for team in &config.teams {
        if let Some(div) = config.divisions.iter().find(|d| d.id == team.division_id) {
            let div_teams_count = config.teams.iter()
                .filter(|t| t.division_id == div.id)
                .count();
            let finals_count = if div.finals_enabled {
                match div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) {
                    crate::model::FinalsRounds::Grand => 1,
                    crate::model::FinalsRounds::Semis => 2,
                    crate::model::FinalsRounds::Quarter => 3,
                    crate::model::FinalsRounds::Eighths => 4,
                }
            } else {
                0
            };
            let games_per_team = match div.mode {
                crate::model::SchedulingMode::HeadToHead => {
                    if div_teams_count < 2 {
                        0
                    } else {
                        div.games_per_team + finals_count
                    }
                }
                crate::model::SchedulingMode::IndividualRun => div.games_per_team,
            };
            let mut count = games_per_team;
            if div.interviews_enabled {
                count += 1;
            }
            if count > slots.len() {
                diagnostics.push(DiagnosticMessage {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Team '{}' (division: '{}') is scheduled for a total of {} activities, which exceeds the {} available time slots.",
                        team.name, div.name, count, slots.len()
                    ),
                    recommendation: Some(format!(
                        "Add at least {} more time slots to the schedule.",
                        count - slots.len()
                    )),
                });
            }
        }
    }

    // 5. Volunteer Slot Capacity Check
    // Calculate total volunteer-slots required
    let mut required_vol_slots = 0;
    for activity in &activities {
        let div_id = activity.division_id();
        if let Some(div) = config.divisions.iter().find(|d| d.id == div_id) {
            required_vol_slots += match activity {
                Activity::Interview { .. } => div.interview_volunteers_required,
                _ => div.volunteers_required,
            };
        }
    }

    // Calculate total volunteer-slots available
    let total_available_vol_slots: usize = volunteers
        .iter()
        .map(|v| v.availabilities.len())
        .sum();

    if required_vol_slots > total_available_vol_slots {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Not enough volunteer capacity. The schedule requires {} volunteer-slots, but you only have {} available (sum of all volunteer availabilities).",
                required_vol_slots, total_available_vol_slots
            ),
            recommendation: Some(
                "Register more volunteers OR edit existing volunteers to increase their available time slots.".to_string()
            ),
        });
    } else if (total_available_vol_slots as f64) < (required_vol_slots as f64) * 1.25 {
        // Tight volunteer capacity warning
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Volunteer availability is tight. You have {} volunteer-slots available for {} required slots. This may make it hard to solve the schedule without conflicts.",
                total_available_vol_slots, required_vol_slots
            ),
            recommendation: Some(
                "Adding 1-2 more volunteers or extending availabilities is recommended to make scheduling easier.".to_string()
            ),
        });
    }

    // 6. Time Slot Specific Volunteer Capacity Check
    // For each time slot, check if the number of available volunteers is enough.
    for slot in slots {
        // Count how many volunteers are available in this slot
        let available_in_slot = volunteers
            .iter()
            .filter(|v| v.availabilities.contains(&slot.id))
            .count();

        // If the number of volunteers is very low in this slot, warn
        if available_in_slot == 0 && required_vol_slots > 0 {
            diagnostics.push(DiagnosticMessage {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Time slot '{} ({}-{})' has ZERO volunteers available.",
                    slot.day, slot.start_time, slot.end_time
                ),
                recommendation: Some("Assign at least 1-2 volunteers to be available during this time slot.".to_string()),
            });
        }
    }

    // 7. Overlapping Time Slots
    for (i, s1) in slots.iter().enumerate() {
        for s2 in slots.iter().skip(i + 1) {
            if s1.day == s2.day {
                let start1 = s1.start_minutes();
                let end1 = start1 + s1.duration_minutes();
                let start2 = s2.start_minutes();
                let end2 = start2 + s2.duration_minutes();

                if start1 < end2 && start2 < end1 && s1.kind == s2.kind {
                    diagnostics.push(DiagnosticMessage {
                        severity: DiagnosticSeverity::Warning,
                        message: format!("Time slots '{}' and '{}' on {} overlap.", s1.id, s2.id, s1.day),
                        recommendation: Some("Adjust the start or end times so slots do not overlap, or ignore if this is intentional.".to_string()),
                    });
                }
            }
        }
    }

    // 8. Aggregate Field Capacity Check
    let mut competition_activities_count = 0;
    let mut interview_activities_count = 0;
    for activity in &activities {
        if matches!(activity, Activity::Interview { .. }) {
            interview_activities_count += 1;
        } else {
            competition_activities_count += 1;
        }
    }

    let comp_fields_count = fields.iter().filter(|f| f.kind == FieldKind::Competition).count();
    let total_competition_field_slots = comp_fields_count * slots.len();
    
    if competition_activities_count > total_competition_field_slots {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Global competition capacity exceeded. Total required games/runs ({}) exceeds total available competition field-slots ({}) ({} fields * {} slots).",
                competition_activities_count, total_competition_field_slots, 
                comp_fields_count, slots.len()
            ),
            recommendation: Some("Add more fields or time slots, or reduce the games per team.".to_string()),
        });
    }

    let int_fields_count = fields.iter().filter(|f| f.kind == FieldKind::Interview).count();
    let total_interview_field_slots = int_fields_count * slots.len();

    if interview_activities_count > total_interview_field_slots {
        diagnostics.push(DiagnosticMessage {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Global interview capacity exceeded. Total required interviews ({}) exceeds total available interview-slots ({}) ({} interview tables * {} slots).",
                interview_activities_count, total_interview_field_slots,
                int_fields_count, slots.len()
            ),
            recommendation: Some("Add more interview tables or time slots.".to_string()),
        });
    }

    // Sort diagnostics: Error > Warning > Info
    diagnostics.sort_by_key(|d| match d.severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
    });

    diagnostics
}

pub fn validate_schedule(config: &TournamentConfig, schedule: &crate::model::Schedule, params: &crate::scheduler::SolverParams) -> Vec<DiagnosticMessage> {
    let assignment_conflicts = crate::scheduler::get_assignment_conflicts(config, schedule, params);
    let mut messages = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for conflicts in assignment_conflicts.values() {
        for conflict in conflicts {
            if seen.insert(conflict.message.clone()) {
                messages.push(DiagnosticMessage {
                    severity: match conflict.severity {
                        crate::scheduler::ConflictSeverity::Error => DiagnosticSeverity::Error,
                        crate::scheduler::ConflictSeverity::Warning => DiagnosticSeverity::Warning,
                    },
                    message: format!("Schedule Conflict: {}", conflict.message),
                    recommendation: Some("Re-run the solver or manually adjust assignments to resolve this conflict.".to_string()),
                });
            }
        }
    }
    messages
}
