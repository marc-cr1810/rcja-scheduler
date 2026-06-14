use crate::model::{Activity, MatchStage, SchedulingMode, TournamentConfig};
use super::utils::sanitize_name;

/// Computes the actual number of round-robin matches that will be generated for
/// a division, accounting for the two-phase selection logic that minimises
/// variance in games per team (pairs deficit teams first, then fills stragglers).
pub fn compute_rr_match_count(num_teams: usize, games_per_team: usize) -> usize {
    if num_teams < 2 {
        return 0;
    }

    let padded_n = if num_teams % 2 == 0 { num_teams } else { num_teams + 1 };
    let num_rounds = padded_n - 1;
    let games_per_round = padded_n / 2;

    // Generate all (home, away) pairs from enough cycles, skipping byes.
    let matches_per_cycle = num_teams * (num_teams - 1) / 2;
    let min_needed = (num_teams * games_per_team + 1) / 2;
    let cycles = (min_needed + matches_per_cycle - 1) / matches_per_cycle.max(1);

    let mut all_pairs: Vec<(usize, usize)> = Vec::new();
    for _ in 0..cycles.max(1) {
        for r in 0..num_rounds {
            for g in 0..games_per_round {
                let (home, away) = if g == 0 {
                    if r % 2 == 0 { (padded_n - 1, r) } else { (r, padded_n - 1) }
                } else {
                    ((r + g) % (padded_n - 1), (r + padded_n - 1 - g) % (padded_n - 1))
                };
                if home < num_teams && away < num_teams {
                    all_pairs.push((home, away));
                }
            }
        }
    }

    // Phase 1: both below quota
    let mut counts = vec![0usize; num_teams];
    let mut total = 0;
    let mut deferred = Vec::new();
    for (h, a) in all_pairs {
        if counts[h] < games_per_team && counts[a] < games_per_team {
            counts[h] += 1;
            counts[a] += 1;
            total += 1;
        } else {
            deferred.push((h, a));
        }
    }

    // Phase 2a: among deferred, both still below
    let mut still_deferred = Vec::new();
    for (h, a) in deferred {
        if counts[h] < games_per_team && counts[a] < games_per_team {
            counts[h] += 1;
            counts[a] += 1;
            total += 1;
        } else {
            still_deferred.push((h, a));
        }
    }

    // Phase 2b: one below
    for (h, a) in still_deferred {
        if counts.iter().all(|&c| c >= games_per_team) {
            break;
        }
        if counts[h] < games_per_team || counts[a] < games_per_team {
            counts[h] += 1;
            counts[a] += 1;
            total += 1;
        }
    }

    total
}


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

    // Generate enough full round-robin cycles to have sufficient candidate matches.
    let matches_per_cycle = n * (n - 1) / 2;
    let min_needed = (n * games_per_team + 1) / 2;
    let cycles_needed = (min_needed + matches_per_cycle - 1) / matches_per_cycle.max(1);

    let mut all_matches = Vec::new();
    for cycle in 0..cycles_needed.max(1) {
        let mut cycle_matches = generate_circle_round_robin(division_id, teams, duration_minutes);
        for m in &mut cycle_matches {
            if let Activity::Match { id, stage, .. } = m {
                *id = id.replacen("_c0_r", &format!("_c{}_r", cycle), 1);
                if let MatchStage::RoundRobin { cycle: c, .. } = stage {
                    *c = cycle;
                }
            }
        }
        all_matches.extend(cycle_matches);
    }

    // Two-phase selection to minimise variance in games per team:
    // Phase 1: Take matches where BOTH teams are below quota (pairs deficit teams).
    // Phase 2: Fill remaining deficits with matches where at least one team needs games.
    let mut team_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut selected = Vec::new();
    let mut deferred = Vec::new();

    // Phase 1: both below quota
    for m in all_matches {
        if let Activity::Match { ref team_a, ref team_b, .. } = m {
            let ca = *team_counts.get(team_a).unwrap_or(&0);
            let cb = *team_counts.get(team_b).unwrap_or(&0);
            if ca < games_per_team && cb < games_per_team {
                *team_counts.entry(team_a.clone()).or_insert(0) += 1;
                *team_counts.entry(team_b.clone()).or_insert(0) += 1;
                selected.push(m);
            } else {
                deferred.push(m);
            }
        }
    }

    // Phase 2a: among deferred, catch any remaining both-below pairings
    let mut still_deferred = Vec::new();
    for m in deferred {
        if let Activity::Match { ref team_a, ref team_b, .. } = m {
            let ca = *team_counts.get(team_a).unwrap_or(&0);
            let cb = *team_counts.get(team_b).unwrap_or(&0);
            if ca < games_per_team && cb < games_per_team {
                *team_counts.entry(team_a.clone()).or_insert(0) += 1;
                *team_counts.entry(team_b.clone()).or_insert(0) += 1;
                selected.push(m);
            } else {
                still_deferred.push(m);
            }
        }
    }

    // Phase 2b: one below quota (fills the last straggler)
    for m in still_deferred {
        if teams.iter().all(|t| *team_counts.get(t).unwrap_or(&0) >= games_per_team) {
            break;
        }
        if let Activity::Match { ref team_a, ref team_b, .. } = m {
            let ca = *team_counts.get(team_a).unwrap_or(&0);
            let cb = *team_counts.get(team_b).unwrap_or(&0);
            if ca < games_per_team || cb < games_per_team {
                *team_counts.entry(team_a.clone()).or_insert(0) += 1;
                *team_counts.entry(team_b.clone()).or_insert(0) += 1;
                selected.push(m);
            }
        }
    }

    selected
}

fn generate_finals_matches(
    division_id: &str,
    rounds: crate::model::FinalsRounds,
    duration_minutes: u32,
    third_place_playoff: bool,
) -> Vec<Activity> {
    let mut matches = Vec::new();
    let prefix = format!("{} ", division_id);

    let push_match = |matches: &mut Vec<Activity>, id: String, team_a: String, team_b: String, stage: MatchStage| {
        matches.push(Activity::Match {
            id,
            team_a,
            team_b,
            division_id: division_id.to_string(),
            duration_minutes,
            stage,
        });
    };

    match rounds {
        crate::model::FinalsRounds::Grand => {
            push_match(&mut matches, format!("{}_gf", division_id),
                format!("{}1st Place", prefix), format!("{}2nd Place", prefix), MatchStage::GrandFinal);
        }
        crate::model::FinalsRounds::Semis => {
            push_match(&mut matches, format!("{}_sf_1", division_id),
                format!("{}1st Place", prefix), format!("{}4th Place", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_sf_2", division_id),
                format!("{}2nd Place", prefix), format!("{}3rd Place", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_gf", division_id),
                format!("{}Winner SF1", prefix), format!("{}Winner SF2", prefix), MatchStage::GrandFinal);
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
                push_match(&mut matches, format!("{}_qf_{}", division_id, i),
                    format!("{}{}", prefix, ta), format!("{}{}", prefix, tb), MatchStage::QuarterFinal);
            }
            push_match(&mut matches, format!("{}_sf_1", division_id),
                format!("{}Winner QF1", prefix), format!("{}Winner QF4", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_sf_2", division_id),
                format!("{}Winner QF2", prefix), format!("{}Winner QF3", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_gf", division_id),
                format!("{}Winner SF1", prefix), format!("{}Winner SF2", prefix), MatchStage::GrandFinal);
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
                push_match(&mut matches, format!("{}_ef_{}", division_id, i),
                    format!("{}{}", prefix, ta_str), format!("{}{}", prefix, tb_str), MatchStage::EighthFinal);
            }
            for i in 1..=4 {
                let (ta, tb) = match i {
                    1 => ("Winner EF1", "Winner EF8"),
                    2 => ("Winner EF2", "Winner EF7"),
                    3 => ("Winner EF3", "Winner EF6"),
                    4 => ("Winner EF4", "Winner EF5"),
                    _ => unreachable!(),
                };
                push_match(&mut matches, format!("{}_qf_{}", division_id, i),
                    format!("{}{}", prefix, ta), format!("{}{}", prefix, tb), MatchStage::QuarterFinal);
            }
            push_match(&mut matches, format!("{}_sf_1", division_id),
                format!("{}Winner QF1", prefix), format!("{}Winner QF4", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_sf_2", division_id),
                format!("{}Winner QF2", prefix), format!("{}Winner QF3", prefix), MatchStage::SemiFinal);
            push_match(&mut matches, format!("{}_gf", division_id),
                format!("{}Winner SF1", prefix), format!("{}Winner SF2", prefix), MatchStage::GrandFinal);
        }
    }

    if third_place_playoff && rounds != crate::model::FinalsRounds::Grand {
        push_match(&mut matches, format!("{}_3pl", division_id),
            format!("{}Loser SF1", prefix), format!("{}Loser SF2", prefix), MatchStage::ThirdPlace);
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
                    stage: MatchStage::RoundRobin { cycle: 0, round: r },
                });
            }
        }
    }

    matches
}
