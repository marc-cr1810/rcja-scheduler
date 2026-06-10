use crate::model::{Activity, SchedulingMode, TournamentConfig};
use super::utils::sanitize_name;

pub fn generate_activities(config: &TournamentConfig) -> Vec<Activity> {
    let mut activities = Vec::new();

    for div in &config.divisions {
        let div_teams: Vec<String> = config
            .teams
            .iter()
            .filter(|t| t.division_id == div.id)
            .map(|t| t.name.clone())
            .collect();

        if div_teams.is_empty() {
            continue;
        }

        match div.mode {
            SchedulingMode::HeadToHead => {
                let mut matches = generate_head_to_head_matches(
                    &div.id,
                    &div_teams,
                    div.games_per_team,
                    div.duration_minutes,
                );
                activities.append(&mut matches);

                if div.finals_enabled {
                    let mut finals_matches = generate_finals_matches(
                        &div.id,
                        div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand),
                        div.finals_duration_minutes.unwrap_or(div.duration_minutes),
                        div.finals_third_place_playoff,
                    );
                    activities.append(&mut finals_matches);
                }
            }
            SchedulingMode::IndividualRun => {
                for team in &div_teams {
                    for r in 1..=div.games_per_team {
                        activities.push(Activity::Run {
                            id: format!("{}_run_{}_{}", div.id, sanitize_name(team), r),
                            team: team.clone(),
                            division_id: div.id.clone(),
                            run_number: r,
                            duration_minutes: div.duration_minutes,
                        });
                    }
                }
            }
        }

        if div.interviews_enabled {
            for team in &div_teams {
                activities.push(Activity::Interview {
                    id: format!("{}_interview_{}", div.id, sanitize_name(team)),
                    team: team.clone(),
                    division_id: div.id.clone(),
                    duration_minutes: div.interview_duration_minutes,
                });
            }
        }
    }

    activities
}

fn generate_head_to_head_matches(
    division_id: &str,
    teams: &[String],
    games_per_team: usize,
    duration_minutes: u32,
) -> Vec<Activity> {
    let n = teams.len();
    if n < 2 {
        return Vec::new();
    }

    let total_matches_needed = (n * games_per_team).div_ceil(2);
    let mut all_matches = Vec::new();
    let mut cycle = 0;

    while all_matches.len() < total_matches_needed {
        let mut cycle_matches = generate_circle_round_robin(division_id, teams, duration_minutes);
        
        for m in &mut cycle_matches {
            if let Activity::Match { id, .. } = m {
                *id = id.replacen("_c0_r", &format!("_c{}_r", cycle), 1);
            }
        }
        
        all_matches.append(&mut cycle_matches);
        cycle += 1;
        
        if cycle > 100 {
            break;
        }
    }

    if all_matches.len() > total_matches_needed {
        all_matches.truncate(total_matches_needed);
    }

    all_matches
}

fn generate_finals_matches(
    division_id: &str,
    rounds: crate::model::FinalsRounds,
    duration_minutes: u32,
    third_place_playoff: bool,
) -> Vec<Activity> {
    let mut matches = Vec::new();
    let prefix = format!("{} ", division_id);

    match rounds {
        crate::model::FinalsRounds::Grand => {
            matches.push(Activity::Match {
                id: format!("{}_gf", division_id),
                team_a: format!("{}1st Place", prefix),
                team_b: format!("{}2nd Place", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
        }
        crate::model::FinalsRounds::Semis => {
            matches.push(Activity::Match {
                id: format!("{}_sf_1", division_id),
                team_a: format!("{}1st Place", prefix),
                team_b: format!("{}4th Place", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_sf_2", division_id),
                team_a: format!("{}2nd Place", prefix),
                team_b: format!("{}3rd Place", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_gf", division_id),
                team_a: format!("{}Winner SF1", prefix),
                team_b: format!("{}Winner SF2", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
        }
        crate::model::FinalsRounds::Quarter => {
            for i in 1..=4 {
                let (ta, tb) = match i {
                    1 => ("1st Place", "8th Place"),
                    2 => ("2nd Place", "7th Place"),
                    3 => ("3rd Place", "6th Place"),
                    4 => ("4th Place", "5th Place"),
                    _ => unreachable!(),
                };
                matches.push(Activity::Match {
                    id: format!("{}_qf_{}", division_id, i),
                    team_a: format!("{}{}", prefix, ta),
                    team_b: format!("{}{}", prefix, tb),
                    division_id: division_id.to_string(),
                    duration_minutes,
                    is_final: true,
                });
            }
            matches.push(Activity::Match {
                id: format!("{}_sf_1", division_id),
                team_a: format!("{}Winner QF1", prefix),
                team_b: format!("{}Winner QF4", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_sf_2", division_id),
                team_a: format!("{}Winner QF2", prefix),
                team_b: format!("{}Winner QF3", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_gf", division_id),
                team_a: format!("{}Winner SF1", prefix),
                team_b: format!("{}Winner SF2", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
        }
        crate::model::FinalsRounds::Eighths => {
            for i in 1..=8 {
                let ta_str = match i {
                    1 => "1st Place".to_string(),
                    2 => "2nd Place".to_string(),
                    3 => "3rd Place".to_string(),
                    _ => format!("{}th Place", i),
                };
                let tb_str = match 17 - i {
                    14 => "14th Place".to_string(),
                    15 => "15th Place".to_string(),
                    16 => "16th Place".to_string(),
                    other => format!("{}th Place", other),
                };
                matches.push(Activity::Match {
                    id: format!("{}_ef_{}", division_id, i),
                    team_a: format!("{}{}", prefix, ta_str),
                    team_b: format!("{}{}", prefix, tb_str),
                    division_id: division_id.to_string(),
                    duration_minutes,
                    is_final: true,
                });
            }
            for i in 1..=4 {
                let (ta, tb) = match i {
                    1 => ("Winner EF1", "Winner EF8"),
                    2 => ("Winner EF2", "Winner EF7"),
                    3 => ("Winner EF3", "Winner EF6"),
                    4 => ("Winner EF4", "Winner EF5"),
                    _ => unreachable!(),
                };
                matches.push(Activity::Match {
                    id: format!("{}_qf_{}", division_id, i),
                    team_a: format!("{}{}", prefix, ta),
                    team_b: format!("{}{}", prefix, tb),
                    division_id: division_id.to_string(),
                    duration_minutes,
                    is_final: true,
                });
            }
            matches.push(Activity::Match {
                id: format!("{}_sf_1", division_id),
                team_a: format!("{}Winner QF1", prefix),
                team_b: format!("{}Winner QF4", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_sf_2", division_id),
                team_a: format!("{}Winner QF2", prefix),
                team_b: format!("{}Winner QF3", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
            matches.push(Activity::Match {
                id: format!("{}_gf", division_id),
                team_a: format!("{}Winner SF1", prefix),
                team_b: format!("{}Winner SF2", prefix),
                division_id: division_id.to_string(),
                duration_minutes,
                is_final: true,
            });
        }
    }

    if third_place_playoff && rounds != crate::model::FinalsRounds::Grand {
        matches.push(Activity::Match {
            id: format!("{}_3pl", division_id),
            team_a: format!("{}Loser SF1", prefix),
            team_b: format!("{}Loser SF2", prefix),
            division_id: division_id.to_string(),
            duration_minutes,
            is_final: true,
        });
    }

    matches
}

fn generate_circle_round_robin(
    division_id: &str,
    teams: &[String],
    duration_minutes: u32,
) -> Vec<Activity> {
    let mut padded_teams = teams.to_vec();
    if !padded_teams.len().is_multiple_of(2) {
        padded_teams.push("__BYE__".to_string());
    }

    let n = padded_teams.len();
    let num_rounds = n - 1;
    let games_per_round = n / 2;
    let mut matches = Vec::new();
    let mut match_idx = 0;

    for r in 0..num_rounds {
        for g in 0..games_per_round {
            let (home, away) = if g == 0 {
                if r % 2 == 0 {
                    (n - 1, r)
                } else {
                    (r, n - 1)
                }
            } else {
                let h = (r + g) % (n - 1);
                let a = (r + n - 1 - g) % (n - 1);
                (h, a)
            };

            let team_h = &padded_teams[home];
            let team_a = &padded_teams[away];

            if team_h != "__BYE__" && team_a != "__BYE__" {
                match_idx += 1;
                matches.push(Activity::Match {
                    id: format!("{}_m_{}_c0_r{}", division_id, match_idx, r),
                    team_a: team_h.clone(),
                    team_b: team_a.clone(),
                    division_id: division_id.to_string(),
                    duration_minutes,
                    is_final: false,
                });
            }
        }
    }

    matches
}
