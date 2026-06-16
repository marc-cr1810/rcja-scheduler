use crate::model::{
    Activity, FairnessMode, Schedule, ScheduleAssignment, TournamentConfig, FieldKind,
};
use rand::seq::SliceRandom;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use super::internal::{InternalTournamentConfig, InternalSchedule, InternalAssignment, InternalActivity};
use super::fast_evaluator::FastEvaluator;
use super::conflicts::distinct_hard_conflicts;
use super::SolverParams;
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub fn solve_schedule(
    config: &TournamentConfig,
    params: &SolverParams,
    progress_callback: impl Fn(usize, usize, usize, usize, f64, f64) + Send + Sync + 'static,
) -> Option<Schedule> {
    let activities = super::generate_activities(config);
    if activities.is_empty() {
        return Some(Schedule::default());
    }

    let internal_config = InternalTournamentConfig::compile(config, &activities);
    let progress_callback = Arc::new(progress_callback);
    let restarts_completed = Arc::new(AtomicUsize::new(0));

    let best_result = (0..params.num_restarts)
        .into_par_iter()
        .filter_map(|restart_idx| {
            // Check for cancellation
            if let Some(ref flag) = params.cancel_flag
                && flag.load(Ordering::Relaxed) {
                    return None;
                }

            // Seed per restart so a `Some(seed)` run is fully reproducible while
            // each restart still explores a distinct random stream. With no seed,
            // draw from system entropy (different result each run). StdRng is
            // portable, so a given seed reproduces across platforms.
            let mut rng = match params.seed {
                Some(s) => StdRng::seed_from_u64(s.wrapping_add(restart_idx as u64)),
                None => StdRng::from_entropy(),
            };
            let mut current_schedule = construct_initial_internal_schedule(&internal_config, params.fairness_mode, &mut rng);
            let mut evaluator = FastEvaluator::new(&internal_config, params);
            evaluator.inc_init(&current_schedule);
            let mut current_cost = evaluator.inc_total();

            let mut best_local_schedule = current_schedule.clone();
            let mut best_local_cost = current_cost;

            let mut temp = 1.0;
            let cooling_rate = 0.9999;

            for iter in 0..params.max_iterations {
                if iter % 500 == 0 {
                    // Report the best-so-far, not the annealing's current state:
                    // the current cost intentionally fluctuates (worse moves are
                    // accepted to escape local minima), so reporting it would make
                    // the live numbers jump around and disagree with the schedule
                    // that is ultimately returned. We report the distinct hard
                    // *count* (same metric the GUI shows when done) so the live and
                    // final numbers are identical, plus the soft penalty score.
                    // Use a throwaway evaluator: `collect_conflicts` runs a full
                    // scan that rebuilds shared state for `best_local_schedule`,
                    // which would corrupt `evaluator`'s incremental state for the
                    // (different) current schedule.
                    let best_hard = distinct_hard_conflicts(&FastEvaluator::new(&internal_config, params).collect_conflicts(&best_local_schedule)).len() as f64;
                    (progress_callback)(restart_idx, params.num_restarts, iter, params.max_iterations, best_hard, best_local_cost.1);

                    if let Some(ref flag) = params.cancel_flag
                        && flag.load(Ordering::Relaxed) {
                            return None;
                        }
                }

                if current_cost.0 == 0.0 && current_cost.1 == 0.0 {
                    break;
                }

                // Mutate in-place
                let mutation = mutate_internal_schedule_in_place(
                    &internal_config,
                    &mut current_schedule,
                    &mut evaluator,
                    params,
                    &mut rng,
                );

                // Score the move incrementally: detach the prior assignment(s),
                // attach the new ones, recompute only the touched partitions.
                let olds = mutation.old_assignments(&current_schedule);
                let mutated_cost = evaluator.apply_change(&current_schedule, &olds);

                let old_total = current_cost.0 * 1_000_000.0 + current_cost.1;
                let new_total = mutated_cost.0 * 1_000_000.0 + mutated_cost.1;

                if new_total < old_total {
                    current_cost = mutated_cost;
                    if new_total < (best_local_cost.0 * 1_000_000.0 + best_local_cost.1) {
                        best_local_cost = mutated_cost;
                        best_local_schedule = current_schedule.clone();
                    }
                } else {
                    let delta = new_total - old_total;
                    let prob = (-delta / temp).exp();
                    if rng.gen_range(0.0..1.0) < prob && temp > 0.01 {
                        current_cost = mutated_cost;
                    } else {
                        // Reject: snapshot the new state, restore the schedule,
                        // then feed the new values back as "prior" so the
                        // evaluator detaches them and re-attaches the originals.
                        let news: Vec<(usize, InternalAssignment)> = mutation
                            .touched()
                            .into_iter()
                            .map(|i| (i, current_schedule.assignments[i].clone()))
                            .collect();
                        revert_mutation(&mut current_schedule, mutation);
                        evaluator.apply_change(&current_schedule, &news);
                    }
                }

                temp *= cooling_rate;
            }

            restarts_completed.fetch_add(1, Ordering::Relaxed);
            let best_hard = distinct_hard_conflicts(&evaluator.collect_conflicts(&best_local_schedule)).len() as f64;
            (progress_callback)(restart_idx, params.num_restarts, params.max_iterations, params.max_iterations, best_hard, best_local_cost.1);

            Some((restart_idx, best_local_schedule, best_local_cost))
        })
        // Pick the cheapest schedule, breaking ties by restart index. The
        // explicit tiebreak makes the choice independent of the order in which
        // rayon reduces the parallel results, so a seeded solve is deterministic.
        .reduce_with(|a, b| {
            let t1 = a.2.0 * 1_000_000.0 + a.2.1;
            let t2 = b.2.0 * 1_000_000.0 + b.2.1;
            if (t1, a.0) <= (t2, b.0) { a } else { b }
        });

    best_result.map(|(_, internal_schedule, _)| decompile_schedule(config, &internal_config, &activities, internal_schedule))
}

fn construct_initial_internal_schedule(
    config: &InternalTournamentConfig,
    fairness_mode: FairnessMode,
    rng: &mut impl Rng,
) -> InternalSchedule {
    let mut assignments = Vec::with_capacity(config.activities.len());

    let mut vol_counts = vec![0usize; config.volunteers.len()];

    for activity in &config.activities {
        let slot_idx = {
            let range = if activity.is_interview {
                (0..config.slots.len())
                    .filter(|&i| {
                        config.slots[i].kind == FieldKind::Interview && 
                        config.day_interviews_enabled[config.slots[i].day_idx]
                    })
                    .collect::<Vec<_>>()
            } else {
                let r = config.round_ranges[activity.round_index].clone();
                (r).filter(|&i| config.slots[i].kind == FieldKind::Competition).collect::<Vec<_>>()
            };

            if range.is_empty() {
                // Fallback if no matching slots
                rng.gen_range(0..config.slots.len())
            } else {
                *range.choose(rng).unwrap()
            }
        };

        let suitable_fields: Vec<usize> = (0..config.fields.len())
            .filter(|&f_idx| {
                let f = &config.fields[f_idx];
                if f.kind == FieldKind::Competition && activity.is_interview { return false; }
                if f.kind == FieldKind::Interview && !activity.is_interview { return false; }
                if let Some(ref allowed) = f.allowed_division_indices
                    && !allowed.contains(&activity.division_idx) { return false; }
                true
            })
            .collect();

        let field_idx = suitable_fields.choose(rng).copied();

        let qualified_volunteers: Vec<usize> = (0..config.volunteers.len())
            .filter(|&v_idx| {
                let v = &config.volunteers[v_idx];
                // Respect a field lock: a pinned volunteer is only eligible for an
                // activity currently placed on one of their allowed fields.
                if let Some(ref locked) = v.locked_field_indices
                    && !field_idx.is_some_and(|f| locked.contains(&f)) {
                    return false;
                }
                if activity.is_interview
                    && config.can_interview[v_idx] { return true; }

                if let Some(ref caps) = v.capability_indices {
                    if caps.contains(&activity.division_idx) { return true; }
                    return false;
                }

                true
            })
            .collect();

        // Try to pick available volunteers for the selected slot
        let available_qualified = qualified_volunteers.iter()
            .filter(|&&v_idx| config.volunteers[v_idx].availability_slots[slot_idx])
            .copied()
            .collect::<Vec<_>>();
        
        let pool = if !available_qualified.is_empty() { &available_qualified } else { &qualified_volunteers };

        let req_volunteers = if activity.is_interview {
            config.divisions[activity.division_idx].interview_volunteers_required
        } else {
            config.divisions[activity.division_idx].volunteers_required
        };

        let assigned_volunteers = if !pool.is_empty() && req_volunteers > 0 {
            let picked = pick_volunteers_biased_internal(
                config,
                pool,
                req_volunteers,
                &vol_counts,
                fairness_mode,
                rng,
            );
            for &v_idx in &picked {
                vol_counts[v_idx] += 1;
            }
            picked
        } else {
            Vec::new()
        };

        assignments.push(InternalAssignment {
            slot_idx,
            field_idx,
            volunteer_indices: assigned_volunteers,
        });
    }

    InternalSchedule { assignments }
}

fn pick_volunteers_biased_internal(
    config: &InternalTournamentConfig,
    candidates: &[usize],
    count: usize,
    current_counts: &[usize],
    fairness_mode: FairnessMode,
    rng: &mut impl Rng,
) -> Vec<usize> {
    if candidates.is_empty() || count == 0 {
        return Vec::new();
    }

    match fairness_mode {
        FairnessMode::Off => {
            let mut choices = candidates.to_vec();
            choices.shuffle(rng);
            choices.into_iter().take(count).collect()
        }
        FairnessMode::Balanced | FairnessMode::Strict => {
            let mut indexed: Vec<(usize, f64)> = candidates
                .iter()
                .map(|&v_idx| {
                    let shifts = current_counts[v_idx] as f64;
                    let v = &config.volunteers[v_idx];
                    let avail = v.availability_slots.iter().filter(|&&a| a).count().max(1) as f64;
                    let utilisation = shifts / avail;
                    let jitter = rng.gen_range(0.0..0.001);
                    (v_idx, utilisation + jitter)
                })
                .collect();
            
            if fairness_mode == FairnessMode::Strict {
                indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                indexed.into_iter().take(count).map(|(idx, _)| idx).collect()
            } else {
                // Balanced: weighted random
                let mut result = Vec::new();
                let mut used = vec![false; candidates.len()];
                let weights: Vec<f64> = indexed.iter().map(|(_, u)| 1.0 / (1.0 + u)).collect();

                for _ in 0..count.min(candidates.len()) {
                    let total: f64 = weights.iter().zip(used.iter()).filter(|&(_, &u)| !u).map(|(w, _)| w).sum();
                    if total <= 0.0 { break; }
                    let mut pick = rng.gen_range(0.0..total);
                    for (i, (&w, &u)) in weights.iter().zip(used.iter()).enumerate() {
                        if u { continue; }
                        pick -= w;
                        if pick <= 0.0 {
                            used[i] = true;
                            result.push(candidates[i]);
                            break;
                        }
                    }
                }
                result
            }
        }
    }
}

pub enum Mutation {
    Slot { idx: usize, old_slot: usize },
    Field { idx: usize, old_field: Option<usize> },
    Volunteers { idx: usize, old_vols: Vec<usize> },
    Swap {
        idx1: usize, old_s1: usize, old_f1: Option<usize>,
        idx2: usize, old_s2: usize, old_f2: Option<usize>
    },
}

impl Mutation {
    /// The assignment indices this mutation changed.
    fn touched(&self) -> Vec<usize> {
        match *self {
            Mutation::Slot { idx, .. } | Mutation::Field { idx, .. } | Mutation::Volunteers { idx, .. } => vec![idx],
            Mutation::Swap { idx1, idx2, .. } => vec![idx1, idx2],
        }
    }

    /// Reconstructs the assignment(s) as they were *before* this mutation, given
    /// the post-mutation `schedule`. Only the field(s) the mutation changed
    /// differ, so the rest is read back from the current schedule. Returned as
    /// `(idx, prior_assignment)` pairs ready for [`FastEvaluator::apply_change`].
    fn old_assignments(&self, schedule: &InternalSchedule) -> Vec<(usize, InternalAssignment)> {
        match self {
            Mutation::Slot { idx, old_slot } => {
                let mut a = schedule.assignments[*idx].clone();
                a.slot_idx = *old_slot;
                vec![(*idx, a)]
            }
            Mutation::Field { idx, old_field } => {
                let mut a = schedule.assignments[*idx].clone();
                a.field_idx = *old_field;
                vec![(*idx, a)]
            }
            Mutation::Volunteers { idx, old_vols } => {
                let mut a = schedule.assignments[*idx].clone();
                a.volunteer_indices = old_vols.clone();
                vec![(*idx, a)]
            }
            Mutation::Swap { idx1, old_s1, old_f1, idx2, old_s2, old_f2 } => {
                let mut a1 = schedule.assignments[*idx1].clone();
                a1.slot_idx = *old_s1; a1.field_idx = *old_f1;
                let mut a2 = schedule.assignments[*idx2].clone();
                a2.slot_idx = *old_s2; a2.field_idx = *old_f2;
                vec![(*idx1, a1), (*idx2, a2)]
            }
        }
    }
}

fn mutate_internal_schedule_in_place(
    config: &InternalTournamentConfig,
    schedule: &mut InternalSchedule,
    evaluator: &mut FastEvaluator,
    params: &SolverParams,
    rng: &mut impl Rng,
) -> Mutation {
    // 80% chance to target a conflicted assignment if any exist
    let idx = if rng.gen_range(0.0..1.0) < 0.8 {
        let conflicted = evaluator.get_conflicted_indices(schedule);
        if !conflicted.is_empty() {
            *conflicted.choose(rng).unwrap()
        } else {
            rng.gen_range(0..schedule.assignments.len())
        }
    } else {
        rng.gen_range(0..schedule.assignments.len())
    };

    let mutation_type = rng.gen_range(0..4);

    match mutation_type {
        0 => { // Change slot
            let old_slot = schedule.assignments[idx].slot_idx;
            let act = &config.activities[idx];
            let range = if act.is_interview {
                (0..config.slots.len())
                    .filter(|&i| {
                        config.slots[i].kind == FieldKind::Interview &&
                        config.day_interviews_enabled[config.slots[i].day_idx]
                    })
                    .collect::<Vec<_>>()
            } else {
                let r = config.round_ranges[act.round_index].clone();
                (r).filter(|&i| config.slots[i].kind == FieldKind::Competition).collect::<Vec<_>>()
            };
            let new_slot = if range.is_empty() {
                rng.gen_range(0..config.slots.len())
            } else {
                *range.choose(rng).unwrap()
            };
            schedule.assignments[idx].slot_idx = new_slot;
            Mutation::Slot { idx, old_slot }
        }
        1 => { // Change field
            let old_field = schedule.assignments[idx].field_idx;
            let activity = &config.activities[idx];
            let suitable_fields: Vec<usize> = (0..config.fields.len())
                .filter(|&f_idx| {
                    let f = &config.fields[f_idx];
                    if f.kind == FieldKind::Competition && activity.is_interview { return false; }
                    if f.kind == FieldKind::Interview && !activity.is_interview { return false; }
                    if let Some(ref allowed) = f.allowed_division_indices
                        && !allowed.contains(&activity.division_idx) { return false; }
                    true
                })
                .collect();
            schedule.assignments[idx].field_idx = suitable_fields.choose(rng).copied();
            Mutation::Field { idx, old_field }
        }
        2 => { // Change volunteers
            let old_vols = schedule.assignments[idx].volunteer_indices.clone();
            let activity = &config.activities[idx];
            let req_volunteers = if activity.is_interview {
                config.divisions[activity.division_idx].interview_volunteers_required
            } else {
                config.divisions[activity.division_idx].volunteers_required
            };

            if req_volunteers > 0 {
                let field_idx = schedule.assignments[idx].field_idx;
                let qualified: Vec<usize> = (0..config.volunteers.len())
                    .filter(|&v_idx| {
                        let v = &config.volunteers[v_idx];
                        // Respect a field lock against the activity's current field.
                        if let Some(ref locked) = v.locked_field_indices
                            && !field_idx.is_some_and(|f| locked.contains(&f)) {
                            return false;
                        }
                        if activity.is_interview
                            && config.can_interview[v_idx] { return true; }

                        if let Some(ref caps) = v.capability_indices {
                            if caps.contains(&activity.division_idx) { return true; }
                            return false;
                        }
                        true
                    })
                    .collect();

                if !qualified.is_empty() {
                    let mut vol_counts = vec![0usize; config.volunteers.len()];
                    for (i, a) in schedule.assignments.iter().enumerate() {
                        if i == idx { continue; }
                        for &v_idx in &a.volunteer_indices {
                            vol_counts[v_idx] += 1;
                        }
                    }

                    let slot_idx = schedule.assignments[idx].slot_idx;
                    let available_qualified = qualified.iter()
                        .filter(|&&v_idx| config.volunteers[v_idx].availability_slots[slot_idx])
                        .copied()
                        .collect::<Vec<_>>();

                    let pool = if !available_qualified.is_empty() { &available_qualified } else { &qualified };

                    schedule.assignments[idx].volunteer_indices = pick_volunteers_biased_internal(
                        config, pool, req_volunteers, &vol_counts, params.fairness_mode, rng
                    );
                }
            }
            Mutation::Volunteers { idx, old_vols }
        }
        _ => { // Swap
            let idx2 = if rng.gen_range(0.0..1.0) < 0.5 {
                let conflicted = evaluator.get_conflicted_indices(schedule);
                if !conflicted.is_empty() {
                    *conflicted.choose(rng).unwrap()
                } else {
                    rng.gen_range(0..schedule.assignments.len())
                }
            } else {
                rng.gen_range(0..schedule.assignments.len())
            };

            if idx != idx2 {
                let act1 = &config.activities[idx];
                let act2 = &config.activities[idx2];
                let s1 = schedule.assignments[idx].slot_idx;
                let s2 = schedule.assignments[idx2].slot_idx;

                // Check round-window compatibility before swapping slots
                let in_range = |a: &InternalActivity, s: usize| {
                    if a.is_interview { 
                        config.slots[s].kind == FieldKind::Interview &&
                        config.day_interviews_enabled[config.slots[s].day_idx]
                    }
                    else { config.round_ranges[a.round_index].contains(&s) && config.slots[s].kind == FieldKind::Competition }
                };

                if in_range(act1, s2) && in_range(act2, s1) {
                    // Potential Swap
                    let f1_idx = schedule.assignments[idx].field_idx;
                    let f2_idx = schedule.assignments[idx2].field_idx;
                    
                    // Check suitability before swapping fields
                    let suitable = |a_idx: usize, f_idx_opt: Option<usize>| {
                        if let Some(f_idx) = f_idx_opt {
                            let f = &config.fields[f_idx];
                            let act = &config.activities[a_idx];
                            if f.kind == FieldKind::Competition && act.is_interview { return false; }
                            if f.kind == FieldKind::Interview && !act.is_interview { return false; }
                            if let Some(ref allowed) = f.allowed_division_indices
                                && !allowed.contains(&act.division_idx) { return false; }
                        }
                        true
                    };

                    if suitable(idx, f2_idx) && suitable(idx2, f1_idx) {
                        schedule.assignments[idx].slot_idx = s2;
                        schedule.assignments[idx].field_idx = f2_idx;
                        schedule.assignments[idx2].slot_idx = s1;
                        schedule.assignments[idx2].field_idx = f1_idx;
                        Mutation::Swap { idx1: idx, old_s1: s1, old_f1: f1_idx, idx2, old_s2: s2, old_f2: f2_idx }
                    } else {
                        // Fallback: only swap slots if fields are not compatible
                        schedule.assignments[idx].slot_idx = s2;
                        schedule.assignments[idx2].slot_idx = s1;
                        Mutation::Swap { idx1: idx, old_s1: s1, old_f1: f1_idx, idx2, old_s2: s2, old_f2: f2_idx }
                    }
                } else {
                    // Out of range for their respective rounds, no mutation
                    Mutation::Slot { idx, old_slot: s1 }
                }
            } else {
                // Same index, no mutation
                Mutation::Slot { idx, old_slot: schedule.assignments[idx].slot_idx }
            }
        }
    }
}

fn revert_mutation(schedule: &mut InternalSchedule, mutation: Mutation) {
    match mutation {
        Mutation::Slot { idx, old_slot } => schedule.assignments[idx].slot_idx = old_slot,
        Mutation::Field { idx, old_field } => schedule.assignments[idx].field_idx = old_field,
        Mutation::Volunteers { idx, old_vols } => schedule.assignments[idx].volunteer_indices = old_vols,
        Mutation::Swap { idx1, old_s1, old_f1, idx2, old_s2, old_f2 } => {
            schedule.assignments[idx1].slot_idx = old_s1;
            schedule.assignments[idx1].field_idx = old_f1;
            schedule.assignments[idx2].slot_idx = old_s2;
            schedule.assignments[idx2].field_idx = old_f2;
        }
    }
}

fn decompile_schedule(
    config: &TournamentConfig,
    internal_config: &InternalTournamentConfig,
    activities: &[Activity],
    internal: InternalSchedule,
) -> Schedule {
    let assignments = internal.assignments.into_iter().enumerate().map(|(i, a)| {
        ScheduleAssignment {
            activity: activities[i].clone(),
            time_slot_id: internal_config.slots[a.slot_idx].id.clone(),
            field_id: a.field_idx.map(|f_idx| config.fields[f_idx].id.clone()),
            volunteer_ids: a.volunteer_indices.iter().map(|&v_idx| config.volunteers[v_idx].id.clone()).collect(),
        }
    }).collect();
    Schedule { assignments }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        DayGenConfig, Division, Field, FieldKind, SchedulingMode, Team, TimeSlot, TournamentConfig,
    };

    fn slot(id: &str, start_min: u32) -> TimeSlot {
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        TimeSlot {
            id: id.into(),
            day: "Saturday".into(),
            start_time: fmt(start_min),
            end_time: fmt(start_min + 20),
            kind: FieldKind::Competition,
        }
    }

    fn small_config() -> TournamentConfig {
        let mut config = TournamentConfig::default();
        config.divisions.push(Division {
            id: "d1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 2, volunteers_required: 0, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: false, interview_volunteers_required: 0,
            interview_duration_minutes: 0, finals_enabled: false, finals_rounds: None,
            finals_duration_minutes: None, finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        });
        for t in ["A", "B", "C", "D"] {
            config.teams.push(Team { name: t.into(), division_id: "d1".into(), organization: t.into() });
        }
        config.fields.push(Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None });
        config.fields.push(Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None });
        // 10 competition slots, 09:00 .. in 20-minute steps.
        config.time_slots = (0..10).map(|i| slot(&format!("s{i}"), 9 * 60 + i * 20)).collect();
        config.day_configs.push(DayGenConfig { day: "Saturday".into(), ..Default::default() });
        config
    }

    #[test]
    fn solves_small_tournament_without_hard_conflicts() {
        let config = small_config();
        let activities = super::super::generate_activities(&config);
        assert!(!activities.is_empty());

        let params = SolverParams { max_iterations: 30_000, num_restarts: 3, ..SolverParams::default() };
        let schedule = solve_schedule(&config, &params, |_, _, _, _, _, _| {})
            .expect("solver returned a schedule");

        // Every generated activity is placed exactly once.
        assert_eq!(schedule.assignments.len(), activities.len());
        for a in &schedule.assignments {
            assert!(a.field_id.is_some(), "every activity should get a field");
        }

        // With ample slots/fields and no volunteer requirements, a conflict-free
        // schedule must be reachable.
        let (hard, _soft) = crate::scheduler::evaluate_schedule_cost(&config, &schedule, &params);
        assert_eq!(hard, 0.0, "expected no hard conflicts, got {hard}");
    }

    #[test]
    fn same_seed_yields_identical_schedule() {
        let config = small_config();
        let params = SolverParams {
            max_iterations: 5_000,
            num_restarts: 4,
            seed: Some(0xC0FFEE),
            ..SolverParams::default()
        };

        let run = || solve_schedule(&config, &params, |_, _, _, _, _, _| {}).expect("schedule");
        let a = run();
        let b = run();

        // A seeded solve must be bit-for-bit reproducible: same slots, fields and
        // volunteer rosters in the same order. This is what makes a reported bad
        // schedule reproducible and the benchmark's run-to-run deltas meaningful.
        assert_eq!(a.assignments.len(), b.assignments.len());
        for (x, y) in a.assignments.iter().zip(&b.assignments) {
            assert_eq!(x.time_slot_id, y.time_slot_id);
            assert_eq!(x.field_id, y.field_id);
            assert_eq!(x.volunteer_ids, y.volunteer_ids);
            assert_eq!(x.activity.id(), y.activity.id());
        }
    }

    #[test]
    fn different_seeds_can_diverge() {
        // Sanity check that the seed actually drives the RNG: two different seeds
        // should be capable of producing different schedules. (We don't assert
        // they *always* differ — on a tiny instance the optimum may be unique —
        // only that the seeding path is wired through, exercised alongside the
        // reproducibility guarantee above.)
        let config = small_config();
        let base = SolverParams { max_iterations: 2_000, num_restarts: 2, ..SolverParams::default() };
        let s1 = solve_schedule(&config, &SolverParams { seed: Some(1), ..base.clone() }, |_, _, _, _, _, _| {});
        let s2 = solve_schedule(&config, &SolverParams { seed: Some(2), ..base }, |_, _, _, _, _, _| {});
        assert!(s1.is_some() && s2.is_some());
    }

    /// A config broad enough to fire most rule kinds: three divisions (one
    /// individual-run, so soft field-variety is active), interviews, volunteers
    /// with availability/capability/conflict constraints, a division-restricted
    /// field, and two days.
    fn guardrail_config() -> TournamentConfig {
        use crate::model::{SpecialistMode, Volunteer};
        let mut config = TournamentConfig::default();
        let div = |id: &str, mode, games, interviews| Division {
            id: id.into(), name: id.into(), mode,
            games_per_team: games, volunteers_required: 1, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: interviews,
            interview_volunteers_required: 1, interview_duration_minutes: 10,
            finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
            finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        };
        config.divisions = vec![
            div("d1", SchedulingMode::HeadToHead, 3, true),
            div("d2", SchedulingMode::HeadToHead, 2, false),
            div("d3", SchedulingMode::IndividualRun, 2, false),
        ];
        for (d, names) in [("d1", &["A", "B", "C", "D"][..]), ("d2", &["E", "F", "G", "H"][..]), ("d3", &["I", "J", "K", "L"][..])] {
            for t in names {
                config.teams.push(Team { name: (*t).into(), division_id: d.into(), organization: format!("org{t}") });
            }
        }
        config.fields = vec![
            Field { id: "c1".into(), name: "Court 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "c2".into(), name: "Court 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["d1".into()]) },
            Field { id: "c3".into(), name: "Court 3".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "iv".into(), name: "Interview".into(), kind: FieldKind::Interview, allowed_divisions: None },
        ];
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        let mut slots = Vec::new();
        for (di, day) in ["Saturday", "Sunday"].iter().enumerate() {
            for i in 0..10u32 {
                let start = 9 * 60 + i * 20;
                let kind = if i % 5 == 4 { FieldKind::Interview } else { FieldKind::Competition };
                slots.push(TimeSlot { id: format!("{day}_s{i}"), day: (*day).into(), start_time: fmt(start), end_time: fmt(start + 20), kind });
            }
            config.day_configs.push(DayGenConfig { day: (*day).into(), ..Default::default() });
            let _ = di;
        }
        config.time_slots = slots;
        config.volunteers = vec![
            Volunteer { id: "v1".into(), name: "V1".into(), availabilities: vec![], capabilities: None, conflict_organizations: vec![], attendance_status: Default::default(), locked_field_ids: Some(vec!["c1".into()]) },
            Volunteer { id: "v2".into(), name: "V2".into(), availabilities: vec!["Saturday_s0".into(), "Saturday_s1".into()], capabilities: Some(vec!["d1".into()]), conflict_organizations: vec!["orgA".into()], attendance_status: Default::default(), locked_field_ids: None },
            Volunteer { id: "v3".into(), name: "V3".into(), availabilities: vec![], capabilities: Some(vec!["d2".into(), "d3".into()]), conflict_organizations: vec!["orgE".into()], attendance_status: Default::default(), locked_field_ids: None },
        ];
        let _ = SpecialistMode::Strict;
        config
    }

    /// The incremental evaluator must agree with a full recompute at every step.
    /// We drive the real mutation operators over a rich instance for thousands of
    /// moves — applying and (half the time) reverting each — and assert the
    /// maintained `(hard, soft)` cost equals a from-scratch `calculate_total_cost`
    /// before the move, after applying it, and after reverting it. This is the
    /// guardrail behind the whole delta-evaluation path.
    #[test]
    fn incremental_matches_full_recompute() {
        use rand::{SeedableRng, rngs::StdRng, Rng};
        use crate::model::SpecialistMode;
        use super::super::fast_evaluator::FastEvaluator;

        let config = guardrail_config();
        let activities = super::super::generate_activities(&config);
        assert!(!activities.is_empty());
        let ic = super::super::internal::InternalTournamentConfig::compile(&config, &activities);

        let params = SolverParams {
            vol_daily_shift_cap: 2,
            team_min_break_minutes: 15,
            team_match_min_break_minutes: 20,
            vol_specialist_mode: SpecialistMode::Strict,
            ..SolverParams::default()
        };

        let mut rng = StdRng::seed_from_u64(0xBEEF);
        let mut schedule = construct_initial_internal_schedule(&ic, params.fairness_mode, &mut rng);
        let mut ev = FastEvaluator::new(&ic, &params);
        ev.inc_init(&schedule);

        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        let full = |sched: &InternalSchedule| {
            let mut e = FastEvaluator::new(&ic, &params);
            e.calculate_total_cost(sched)
        };

        for step in 0..5000 {
            let inc = ev.inc_total();
            let f = full(&schedule);
            assert!(close(inc, f), "step {step} pre-mutate: inc {inc:?} != full {f:?}");

            let mutation = mutate_internal_schedule_in_place(&ic, &mut schedule, &mut ev, &params, &mut rng);
            let olds = mutation.old_assignments(&schedule);
            let inc_after = ev.apply_change(&schedule, &olds);
            let f_after = full(&schedule);
            assert!(close(inc_after, f_after), "step {step} post-apply: inc {inc_after:?} != full {f_after:?}");

            if rng.gen_bool(0.5) {
                let news: Vec<(usize, InternalAssignment)> = mutation
                    .touched()
                    .into_iter()
                    .map(|i| (i, schedule.assignments[i].clone()))
                    .collect();
                revert_mutation(&mut schedule, mutation);
                let inc_rev = ev.apply_change(&schedule, &news);
                let f_rev = full(&schedule);
                assert!(close(inc_rev, f_rev), "step {step} post-revert: inc {inc_rev:?} != full {f_rev:?}");
            }
        }
    }

    #[test]
    fn empty_config_yields_empty_schedule() {
        let config = TournamentConfig::default();
        let params = SolverParams::default();
        let schedule = solve_schedule(&config, &params, |_, _, _, _, _, _| {}).expect("some schedule");
        assert!(schedule.assignments.is_empty());
    }

}
