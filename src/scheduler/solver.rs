use super::SolverParams;
use super::cells::{CellGrid, FieldOccupancy};
use super::conflicts::distinct_hard_conflicts;
use super::fast_evaluator::FastEvaluator;
use super::internal::{InternalAssignment, InternalSchedule, InternalTournamentConfig};
use crate::model::{
    Activity, FairnessMode, FieldKind, Schedule, ScheduleAssignment, TournamentConfig,
};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
                && flag.load(Ordering::Relaxed)
            {
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
            let mut current_schedule =
                construct_seed_schedule(&internal_config, params.fairness_mode, &mut rng);
            let mut evaluator = FastEvaluator::new(&internal_config, params);
            evaluator.inc_init(&current_schedule);
            let mut current_cost = evaluator.inc_total();

            // Cell-model move state: a precomputed cell context and a live field
            // occupancy kept in lock-step with `current_schedule`. Relocate/swap
            // moves only ever target free cells, so field double-booking can never
            // be (re)introduced — it is structurally impossible, not penalised.
            let move_ctx = MoveCtx::build(&internal_config);
            let mut occ = FieldOccupancy::from_schedule(&internal_config, &current_schedule);
            // Whether the seed placed everything without a field clash. When true
            // (the feasible case), the move set guarantees the result stays
            // overlap-free; over-constrained instances may start (and remain) with
            // unavoidable overlaps, so the end-of-restart invariant is conditional.
            let seed_overlap_free = !occ.has_overlap();

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
                    let best_hard = distinct_hard_conflicts(
                        &FastEvaluator::new(&internal_config, params)
                            .collect_conflicts(&best_local_schedule),
                    )
                    .len() as f64;
                    (progress_callback)(
                        restart_idx,
                        params.num_restarts,
                        iter,
                        params.max_iterations,
                        best_hard,
                        best_local_cost.1,
                    );

                    if let Some(ref flag) = params.cancel_flag
                        && flag.load(Ordering::Relaxed)
                    {
                        return None;
                    }
                }

                if current_cost.0 == 0.0 && current_cost.1 == 0.0 {
                    break;
                }

                // Mutate in-place. The move updates both the schedule and the
                // live occupancy together, preserving the no-overlap invariant.
                let mutation = mutate_internal_schedule_in_place(
                    &internal_config,
                    &move_ctx,
                    &mut current_schedule,
                    &mut occ,
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
                        revert_mutation(
                            &internal_config,
                            &mut current_schedule,
                            &mut occ,
                            mutation,
                        );
                        evaluator.apply_change(&current_schedule, &news);
                    }
                }

                temp *= cooling_rate;
            }

            restarts_completed.fetch_add(1, Ordering::Relaxed);
            let best_hard =
                distinct_hard_conflicts(&evaluator.collect_conflicts(&best_local_schedule)).len()
                    as f64;
            (progress_callback)(
                restart_idx,
                params.num_restarts,
                params.max_iterations,
                params.max_iterations,
                best_hard,
                best_local_cost.1,
            );

            // Structural invariant (cell model): a feasible seed must yield an
            // overlap-free schedule — relocate/swap only ever target free cells,
            // so a field double-booking can never be introduced. Checked in debug/
            // test builds; free in release. (`FieldDoubleBooked` still lives in the
            // evaluator to validate *user-edited / imported* schedules, which the
            // drag-and-drop editor can dirty; the guarantee here is about *solver
            // output*.)
            debug_assert!(
                !seed_overlap_free
                    || !FieldOccupancy::from_schedule(&internal_config, &best_local_schedule)
                        .has_overlap(),
                "cell-model invariant violated: feasible seed produced a field double-booking"
            );

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

    best_result.map(|(_, internal_schedule, _)| {
        decompile_schedule(config, &internal_config, &activities, internal_schedule)
    })
}

/// The original random constructor. Superseded as the live seed by
/// [`construct_seed_schedule`]; retained because the incremental-evaluator
/// guardrail test drives mutations from a deliberately messy random start.
#[allow(dead_code)]
fn construct_initial_internal_schedule(
    config: &InternalTournamentConfig,
    fairness_mode: FairnessMode,
    rng: &mut impl Rng,
) -> InternalSchedule {
    let mut assignments = Vec::with_capacity(config.activities.len());

    let mut vol_counts = vec![0usize; config.volunteers.len()];

    for activity in config.activities.iter() {
        let slot_idx = {
            let range = if activity.is_interview {
                (0..config.slots.len())
                    .filter(|&i| {
                        config.slots[i].kind == FieldKind::Interview
                            && config.day_interviews_enabled[config.slots[i].day_idx]
                    })
                    .collect::<Vec<_>>()
            } else {
                (0..config.slots.len())
                    .filter(|&i| config.slots[i].kind == FieldKind::Competition)
                    .collect::<Vec<_>>()
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
                if f.kind == FieldKind::Competition && activity.is_interview {
                    return false;
                }
                if f.kind == FieldKind::Interview && !activity.is_interview {
                    return false;
                }
                if let Some(ref allowed) = f.allowed_division_indices
                    && !allowed.contains(&activity.division_idx)
                {
                    return false;
                }
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
                    && !field_idx.is_some_and(|f| locked.contains(&f))
                {
                    return false;
                }
                if activity.is_interview && config.can_interview[v_idx] {
                    return true;
                }

                if let Some(ref caps) = v.capability_indices {
                    if caps.contains(&activity.division_idx) {
                        return true;
                    }
                    return false;
                }

                true
            })
            .collect();

        // Try to pick available volunteers for the selected slot
        let available_qualified = qualified_volunteers
            .iter()
            .filter(|&&v_idx| config.volunteers[v_idx].availability_slots[slot_idx])
            .copied()
            .collect::<Vec<_>>();

        let pool = if !available_qualified.is_empty() {
            &available_qualified
        } else {
            &qualified_volunteers
        };

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

/// Earliest free, usable cell for `activity_idx` scanning chronological
/// `slots[from..]` (slot indices are pre-sorted by day then start). Fields are
/// scanned in the caller's (shuffled) `field_order` for spread/variety. Returns
/// `(slot_idx, field_idx)`, or `None` if every remaining cell is busy.
fn find_free_cell(
    config: &InternalTournamentConfig,
    grid: &CellGrid,
    occ: &FieldOccupancy,
    activity_idx: usize,
    slots: &[usize],
    from: usize,
    field_order: &[usize],
) -> Option<(usize, usize)> {
    let dc = config.activities[activity_idx].duration_class;
    let start = from.min(slots.len());
    for &slot in &slots[start..] {
        for &f in field_order {
            if grid.activity_can_use(config, activity_idx, f, slot)
                && occ.is_free(config, f, slot, dc)
            {
                return Some((slot, f));
            }
        }
    }
    None
}

/// Any usable cell ignoring occupancy — the over-constrained fallback so an
/// activity is always placed even when no field is free (zero hard conflicts is
/// unreachable there anyway; the local-search phase minimises the damage).
fn any_usable_cell(
    config: &InternalTournamentConfig,
    grid: &CellGrid,
    activity_idx: usize,
    slots: &[usize],
    field_order: &[usize],
) -> Option<(usize, usize)> {
    for &slot in slots {
        for &f in field_order {
            if grid.activity_can_use(config, activity_idx, f, slot) {
                return Some((slot, f));
            }
        }
    }
    None
}

/// Constructive seed for the local search (Phase 2 of the cell-model rewrite).
///
/// Places every activity into a **free** cell via the Phase 1 occupancy gate, so
/// the seed has **zero field double-bookings** by construction. Competition
/// matches are laid out **per division, round by round**, each round taking a
/// contiguous chronological window proportional to its size: windows therefore
/// span the whole timeline (including later days, fixing the "Sunday empty"
/// complaint) and each round's window precedes the next, which gives **per-team
/// round order for free** (a team plays once per round, in window order). A
/// per-team "earliest next slot" cursor keeps a team's matches strictly advancing
/// (a coarse recharge gap). Interviews then fill the earliest free interview
/// cells, and volunteers are rostered in a final fairness-biased pass.
///
/// The result is a valid, evenly-spread starting point; the move set (Phase 3)
/// compacts it — overlapping rounds only where teams have finished — without ever
/// reintroducing a field clash.
fn construct_seed_schedule(
    config: &InternalTournamentConfig,
    fairness_mode: FairnessMode,
    rng: &mut impl Rng,
) -> InternalSchedule {
    let grid = CellGrid::build(config);
    let mut occ = FieldOccupancy::new(config.fields.len(), config.num_total_buckets);
    let n = config.activities.len();
    let mut slot_of = vec![0usize; n];
    let mut field_of: Vec<Option<usize>> = vec![None; n];

    // Chronological slot/field index lists (config.slots is sorted by day,start).
    let comp_slots: Vec<usize> = (0..config.slots.len())
        .filter(|&s| config.slots[s].kind == FieldKind::Competition)
        .collect();
    let int_slots: Vec<usize> = (0..config.slots.len())
        .filter(|&s| config.slots[s].kind == FieldKind::Interview)
        .collect();
    let comp_fields: Vec<usize> = (0..config.fields.len())
        .filter(|&f| config.fields[f].kind == FieldKind::Competition)
        .collect();
    let int_fields: Vec<usize> = (0..config.fields.len())
        .filter(|&f| config.fields[f].kind == FieldKind::Interview)
        .collect();

    // Position of each competition slot in the chronological list, for the
    // per-team "earliest next" cursor.
    let comp_pos: BTreeMap<usize, usize> = comp_slots
        .iter()
        .enumerate()
        .map(|(p, &s)| (s, p))
        .collect();

    // Reserve a compact finals block at the very end of the event: one band per
    // stage level (GF and 3rd-place share the last band, then SF, then QF, then
    // EF). All divisions' matches at the same stage land in the same band (they
    // are independent across divisions), so finals pack into the last few slots
    // instead of straddling earlier time / a lunch break. Round-robin then fills
    // only the slots *before* that reserved tail.
    let stage_offset = |stage: usize| -> usize {
        match stage {
            4 | 5 => 0, // 3rd place / grand final
            3 => 1,     // semi
            2 => 2,     // quarter
            1 => 3,     // eighth
            _ => 0,
        }
    };
    let finals_depth = config
        .activities
        .iter()
        .filter(|a| a.is_final && !a.is_interview)
        .map(|a| stage_offset(a.stage) + 1)
        .max()
        .unwrap_or(0);
    let rr_region = comp_slots.len().saturating_sub(finals_depth);
    let rr_slots: &[usize] = &comp_slots[..rr_region];

    // Running per-field placement count, shared across divisions so shared fields
    // reflect their combined load. Field choice prefers the least-loaded field
    // (below), seeding an already-balanced layout for the local search to keep.
    let mut field_load = vec![0u32; config.fields.len()];

    // Seed divisions with the fewest usable fields first. A constrained division
    // (e.g. one sharing only two fields with another) claims its cells before a
    // field-rich division floods them, so shared fields don't end up carrying two
    // divisions' worth of matches while dedicated fields sit lighter.
    let comp_field_count = |div_idx: usize| {
        comp_fields
            .iter()
            .filter(|&&f| {
                config.fields[f]
                    .allowed_division_indices
                    .as_ref()
                    .map_or(true, |a| a.contains(&div_idx))
            })
            .count()
    };
    let mut div_order: Vec<usize> = (0..config.divisions.len()).collect();
    div_order.sort_by_key(|&d| comp_field_count(d));

    // ---- Round-robin: per division, round-banded across the pre-finals region ----
    for &div_idx in &div_order {
        let mut by_round: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (ai, a) in config.activities.iter().enumerate() {
            if !a.is_interview && !a.is_final && a.division_idx == div_idx {
                by_round.entry(a.round_index).or_default().push(ai);
            }
        }
        if by_round.is_empty() {
            continue;
        }
        let rounds: Vec<usize> = by_round.keys().copied().collect();
        let total: usize = by_round.values().map(|v| v.len()).sum::<usize>().max(1);
        let nslots = rr_slots.len().max(1);

        // Contiguous chronological window start per round, sized in proportion to
        // the round's match count so the rounds tile the whole pre-finals region.
        let mut win_start = vec![0usize; rounds.len()];
        let mut cursor = 0usize;
        for (i, r) in rounds.iter().enumerate() {
            win_start[i] = cursor.min(nslots.saturating_sub(1));
            let share =
                ((by_round[r].len() as f64 / total as f64) * nslots as f64).round() as usize;
            cursor = (cursor + share.max(1)).min(nslots);
        }

        let mut field_order = comp_fields.clone();
        field_order.shuffle(rng);

        let mut team_next: BTreeMap<usize, usize> = BTreeMap::new();

        for (i, r) in rounds.iter().enumerate() {
            let mut matches = by_round[r].clone();
            matches.shuffle(rng);
            for ai in matches {
                let act = &config.activities[ai];
                let min_pos = act
                    .team_indices
                    .iter()
                    .filter_map(|t| team_next.get(t))
                    .copied()
                    .max()
                    .unwrap_or(0);
                let from = win_start[i].max(min_pos);
                // Order fields least-loaded-first (shuffled tiebreak) so the
                // earliest free cell `find_free_cell` returns also lands on the
                // emptiest field, balancing per-field workload from the start.
                field_order.shuffle(rng);
                field_order.sort_by_key(|&f| field_load[f]);
                // Prefer this round's window in the pre-finals region; spill forward
                // there; then anywhere free (incl. the finals tail) as a last resort.
                let placed = find_free_cell(config, &grid, &occ, ai, rr_slots, from, &field_order)
                    .or_else(|| {
                        find_free_cell(config, &grid, &occ, ai, rr_slots, min_pos, &field_order)
                    })
                    .or_else(|| {
                        find_free_cell(config, &grid, &occ, ai, &comp_slots, 0, &field_order)
                    })
                    .or_else(|| any_usable_cell(config, &grid, ai, &comp_slots, &field_order));
                if let Some((slot, f)) = placed {
                    slot_of[ai] = slot;
                    field_of[ai] = Some(f);
                    occ.place(config, f, slot, act.duration_class);
                    field_load[f] += 1;
                    let next = comp_pos.get(&slot).copied().unwrap_or(0) + 1;
                    for &t in &act.team_indices {
                        team_next.insert(t, next);
                    }
                } else if let Some(&s) = comp_slots.first() {
                    slot_of[ai] = s;
                    field_of[ai] = comp_fields.first().copied();
                }
            }
        }
    }

    // ---- Finals: compact block at the very end, stage-banded across divisions ----
    {
        let mut field_order = comp_fields.clone();
        field_order.shuffle(rng);
        // Competition fields available to each division, so the placement can go
        // most-constrained-division-first within each stage band: a division with
        // few (often shared) fields must claim its cells before a field-rich
        // division grabs them. Without this, e.g. a 2-field division that shares
        // fields with a 6-field one gets scattered out of the finals block.
        let div_field_count: Vec<usize> = (0..config.divisions.len())
            .map(|d| {
                comp_fields
                    .iter()
                    .filter(|&&f| {
                        config.fields[f]
                            .allowed_division_indices
                            .as_ref()
                            .is_none_or(|a| a.contains(&d))
                    })
                    .count()
            })
            .collect();
        let mut finals: Vec<usize> = (0..n)
            .filter(|&i| config.activities[i].is_final && !config.activities[i].is_interview)
            .collect();
        finals.sort_by_key(|&i| {
            let a = &config.activities[i];
            (stage_offset(a.stage), div_field_count[a.division_idx])
        });
        for ai in finals {
            let dc = config.activities[ai].duration_class;
            let off = stage_offset(config.activities[ai].stage);
            // Target the stage's reserved band; if it is full, spill toward earlier
            // bands (only happens when a stage overflows the available fields).
            let target = comp_slots.len().saturating_sub(1 + off);
            let mut placed = None;
            let mut p = target as isize;
            while p >= 0 {
                let slot = comp_slots[p as usize];
                for &f in &field_order {
                    if grid.activity_can_use(config, ai, f, slot)
                        && occ.is_free(config, f, slot, dc)
                    {
                        placed = Some((slot, f));
                        break;
                    }
                }
                if placed.is_some() {
                    break;
                }
                p -= 1;
            }
            let placed =
                placed.or_else(|| any_usable_cell(config, &grid, ai, &comp_slots, &field_order));
            if let Some((slot, f)) = placed {
                slot_of[ai] = slot;
                field_of[ai] = Some(f);
                occ.place(config, f, slot, dc);
            } else if let Some(&s) = comp_slots.last() {
                slot_of[ai] = s;
                field_of[ai] = comp_fields.first().copied();
            }
        }
    }

    // ---- Interviews: earliest free interview cell, shuffled for spread ----
    {
        let mut field_order = int_fields.clone();
        field_order.shuffle(rng);
        let mut int_acts: Vec<usize> = (0..n)
            .filter(|&i| config.activities[i].is_interview)
            .collect();
        int_acts.shuffle(rng);
        for ai in int_acts {
            let placed = find_free_cell(config, &grid, &occ, ai, &int_slots, 0, &field_order)
                .or_else(|| any_usable_cell(config, &grid, ai, &int_slots, &field_order));
            if let Some((slot, f)) = placed {
                slot_of[ai] = slot;
                field_of[ai] = Some(f);
                occ.place(config, f, slot, config.activities[ai].duration_class);
            } else if let Some(&s) = int_slots.first() {
                slot_of[ai] = s;
                field_of[ai] = int_fields.first().copied();
            }
        }
    }

    // ---- Volunteers: one fairness-biased pass, given each activity's cell ----
    let mut vol_counts = vec![0usize; config.volunteers.len()];
    let mut volunteer_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    for ai in 0..n {
        let act = &config.activities[ai];
        let req = if act.is_interview {
            config.divisions[act.division_idx].interview_volunteers_required
        } else {
            config.divisions[act.division_idx].volunteers_required
        };
        if req == 0 {
            continue;
        }
        let slot = slot_of[ai];
        let field_idx = field_of[ai];
        let qualified: Vec<usize> = (0..config.volunteers.len())
            .filter(|&v| {
                let vv = &config.volunteers[v];
                if let Some(ref locked) = vv.locked_field_indices
                    && !field_idx.is_some_and(|f| locked.contains(&f))
                {
                    return false;
                }
                if act.is_interview && config.can_interview[v] {
                    return true;
                }
                if let Some(ref caps) = vv.capability_indices {
                    return caps.contains(&act.division_idx);
                }
                true
            })
            .collect();
        if qualified.is_empty() {
            continue;
        }
        let avail: Vec<usize> = qualified
            .iter()
            .copied()
            .filter(|&v| config.volunteers[v].availability_slots[slot])
            .collect();
        let pool = if !avail.is_empty() {
            &avail
        } else {
            &qualified
        };
        let picked =
            pick_volunteers_biased_internal(config, pool, req, &vol_counts, fairness_mode, rng);
        for &v in &picked {
            vol_counts[v] += 1;
        }
        volunteer_of[ai] = picked;
    }

    let assignments = (0..n)
        .map(|i| InternalAssignment {
            slot_idx: slot_of[i],
            field_idx: field_of[i],
            volunteer_indices: std::mem::take(&mut volunteer_of[i]),
        })
        .collect();
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
                indexed
                    .into_iter()
                    .take(count)
                    .map(|(idx, _)| idx)
                    .collect()
            } else {
                // Balanced: weighted random
                let mut result = Vec::new();
                let mut used = vec![false; candidates.len()];
                let weights: Vec<f64> = indexed.iter().map(|(_, u)| 1.0 / (1.0 + u)).collect();

                for _ in 0..count.min(candidates.len()) {
                    let total: f64 = weights
                        .iter()
                        .zip(used.iter())
                        .filter(|&(_, &u)| !u)
                        .map(|(w, _)| w)
                        .sum();
                    if total <= 0.0 {
                        break;
                    }
                    let mut pick = rng.gen_range(0.0..total);
                    for (i, (&w, &u)) in weights.iter().zip(used.iter()).enumerate() {
                        if u {
                            continue;
                        }
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

/// Precomputed, restart-stable context for the cell-model moves: the cell grid
/// plus chronological competition / interview slot lists and the field lists of
/// each kind. Built once per restart so each move avoids re-deriving them.
struct MoveCtx {
    grid: CellGrid,
    comp_slots: Vec<usize>,
    int_slots: Vec<usize>,
    comp_fields: Vec<usize>,
    int_fields: Vec<usize>,
}

impl MoveCtx {
    fn build(config: &InternalTournamentConfig) -> Self {
        let comp_slots = (0..config.slots.len())
            .filter(|&s| config.slots[s].kind == FieldKind::Competition)
            .collect();
        let int_slots = (0..config.slots.len())
            .filter(|&s| config.slots[s].kind == FieldKind::Interview)
            .collect();
        let comp_fields = (0..config.fields.len())
            .filter(|&f| config.fields[f].kind == FieldKind::Competition)
            .collect();
        let int_fields = (0..config.fields.len())
            .filter(|&f| config.fields[f].kind == FieldKind::Interview)
            .collect();
        Self {
            grid: CellGrid::build(config),
            comp_slots,
            int_slots,
            comp_fields,
            int_fields,
        }
    }

    fn slots_fields_for(&self, is_interview: bool) -> (&[usize], &[usize]) {
        if is_interview {
            (&self.int_slots, &self.int_fields)
        } else {
            (&self.comp_slots, &self.comp_fields)
        }
    }

    /// A free, usable cell for `activity_idx`: random sampling first, then a
    /// scan from a random offset. `None` only when the activity is fully boxed in
    /// (no free field anywhere — an over-constrained instance). The caller must
    /// have already freed the activity's own current cell from `occ` so a same-
    /// slot field change can be found.
    fn random_free_cell(
        &self,
        config: &InternalTournamentConfig,
        occ: &FieldOccupancy,
        activity_idx: usize,
        field_load: &[u32],
        rng: &mut impl Rng,
    ) -> Option<(usize, usize)> {
        let act = &config.activities[activity_idx];
        let (slots, fields) = self.slots_fields_for(act.is_interview);
        if slots.is_empty() || fields.is_empty() {
            return None;
        }
        let dc = act.duration_class;
        let usable_free = |slot: usize, field: usize| {
            self.grid
                .activity_can_use(config, activity_idx, field, slot)
                && occ.is_free(config, field, slot, dc)
        };
        // Pick a random time, then the least-loaded eligible field free at that
        // time (random tiebreak). Time stays freely explored while field choice
        // actively evens out per-field workload, so balance is driven by the move
        // set rather than left entirely to the soft balance penalty to recover.
        for _ in 0..24 {
            let slot = *slots.choose(rng).unwrap();
            let mut best: Option<usize> = None;
            let mut best_load = u32::MAX;
            for &field in fields {
                if !usable_free(slot, field) {
                    continue;
                }
                let load = field_load.get(field).copied().unwrap_or(0);
                if load < best_load || (load == best_load && rng.gen_bool(0.5)) {
                    best_load = load;
                    best = Some(field);
                }
            }
            if let Some(field) = best {
                return Some((slot, field));
            }
        }
        let n = slots.len();
        let off = rng.gen_range(0..n);
        for k in 0..n {
            let slot = slots[(off + k) % n];
            for &field in fields {
                if usable_free(slot, field) {
                    return Some((slot, field));
                }
            }
        }
        None
    }
}

/// One move in the cell model. `Relocate` and `Swap` only ever land activities on
/// free cells, so they can never create a field double-booking; `Volunteers` is
/// occupancy-neutral. A move that finds nothing to do is encoded as a `Relocate`
/// whose old cell equals the current one (a no-op for both the evaluator and the
/// occupancy revert).
pub enum Mutation {
    Relocate {
        idx: usize,
        old_slot: usize,
        old_field: Option<usize>,
    },
    Volunteers {
        idx: usize,
        old_vols: Vec<usize>,
    },
    Swap {
        idx1: usize,
        old_s1: usize,
        old_f1: Option<usize>,
        idx2: usize,
        old_s2: usize,
        old_f2: Option<usize>,
    },
}

impl Mutation {
    /// The assignment indices this mutation changed.
    fn touched(&self) -> Vec<usize> {
        match *self {
            Mutation::Relocate { idx, .. } | Mutation::Volunteers { idx, .. } => vec![idx],
            Mutation::Swap { idx1, idx2, .. } => vec![idx1, idx2],
        }
    }

    /// Reconstructs the assignment(s) as they were *before* this mutation, given
    /// the post-mutation `schedule`. Only the fields the mutation changed differ,
    /// so the rest is read back from the current schedule. Returned as
    /// `(idx, prior_assignment)` pairs ready for [`FastEvaluator::apply_change`].
    fn old_assignments(&self, schedule: &InternalSchedule) -> Vec<(usize, InternalAssignment)> {
        match self {
            Mutation::Relocate {
                idx,
                old_slot,
                old_field,
            } => {
                let mut a = schedule.assignments[*idx].clone();
                a.slot_idx = *old_slot;
                a.field_idx = *old_field;
                vec![(*idx, a)]
            }
            Mutation::Volunteers { idx, old_vols } => {
                let mut a = schedule.assignments[*idx].clone();
                a.volunteer_indices = old_vols.clone();
                vec![(*idx, a)]
            }
            Mutation::Swap {
                idx1,
                old_s1,
                old_f1,
                idx2,
                old_s2,
                old_f2,
            } => {
                let mut a1 = schedule.assignments[*idx1].clone();
                a1.slot_idx = *old_s1;
                a1.field_idx = *old_f1;
                let mut a2 = schedule.assignments[*idx2].clone();
                a2.slot_idx = *old_s2;
                a2.field_idx = *old_f2;
                vec![(*idx1, a1), (*idx2, a2)]
            }
        }
    }
}

/// Picks an assignment index, biased (80%) toward one currently in a hard
/// conflict so the search spends its effort where it matters (min-conflicts).
fn pick_biased_index(
    schedule: &InternalSchedule,
    evaluator: &mut FastEvaluator,
    rng: &mut impl Rng,
    bias: f64,
) -> usize {
    if rng.gen_range(0.0..1.0) < bias {
        let conflicted = evaluator.get_conflicted_indices(schedule);
        if !conflicted.is_empty() {
            return *conflicted.choose(rng).unwrap();
        }
    }
    rng.gen_range(0..schedule.assignments.len())
}

fn mutate_internal_schedule_in_place(
    config: &InternalTournamentConfig,
    ctx: &MoveCtx,
    schedule: &mut InternalSchedule,
    occ: &mut FieldOccupancy,
    evaluator: &mut FastEvaluator,
    params: &SolverParams,
    rng: &mut impl Rng,
) -> Mutation {
    let idx = pick_biased_index(schedule, evaluator, rng, 0.8);

    // Encodes "nothing changed" as a self-relocate, so the evaluator sees a zero
    // delta and the occupancy revert is a no-op.
    let noop = |schedule: &InternalSchedule| Mutation::Relocate {
        idx,
        old_slot: schedule.assignments[idx].slot_idx,
        old_field: schedule.assignments[idx].field_idx,
    };

    // 0,1 => relocate, 2 => volunteers, 3 => swap.
    match rng.gen_range(0..4) {
        0 | 1 => {
            // Relocate: free the activity's own cell, then place it on a new free
            // cell. Anywhere usable is allowed — round order / spacing / dispersion
            // are penalties the search optimises, not placement gates.
            let old_slot = schedule.assignments[idx].slot_idx;
            let old_field = schedule.assignments[idx].field_idx;
            let dc = config.activities[idx].duration_class;
            if let Some(f) = old_field {
                occ.remove(config, f, old_slot, dc);
            }

            match ctx.random_free_cell(config, occ, idx, evaluator.field_total_loads(), rng) {
                Some((slot, field)) => {
                    schedule.assignments[idx].slot_idx = slot;
                    schedule.assignments[idx].field_idx = Some(field);
                    occ.place(config, field, slot, dc);
                    Mutation::Relocate {
                        idx,
                        old_slot,
                        old_field,
                    }
                }
                None => {
                    // Boxed in: restore and do nothing.
                    if let Some(f) = old_field {
                        occ.place(config, f, old_slot, dc);
                    }
                    Mutation::Relocate {
                        idx,
                        old_slot,
                        old_field,
                    }
                }
            }
        }
        2 => {
            // Re-roster volunteers (occupancy-neutral).
            let old_vols = schedule.assignments[idx].volunteer_indices.clone();
            let activity = &config.activities[idx];
            let req = if activity.is_interview {
                config.divisions[activity.division_idx].interview_volunteers_required
            } else {
                config.divisions[activity.division_idx].volunteers_required
            };
            if req > 0 {
                let field_idx = schedule.assignments[idx].field_idx;
                let qualified: Vec<usize> = (0..config.volunteers.len())
                    .filter(|&v_idx| {
                        let v = &config.volunteers[v_idx];
                        if let Some(ref locked) = v.locked_field_indices
                            && !field_idx.is_some_and(|f| locked.contains(&f))
                        {
                            return false;
                        }
                        if activity.is_interview && config.can_interview[v_idx] {
                            return true;
                        }
                        if let Some(ref caps) = v.capability_indices {
                            return caps.contains(&activity.division_idx);
                        }
                        true
                    })
                    .collect();

                if !qualified.is_empty() {
                    let mut vol_counts = vec![0usize; config.volunteers.len()];
                    for (i, a) in schedule.assignments.iter().enumerate() {
                        if i == idx {
                            continue;
                        }
                        for &v_idx in &a.volunteer_indices {
                            vol_counts[v_idx] += 1;
                        }
                    }
                    let slot_idx = schedule.assignments[idx].slot_idx;
                    let available: Vec<usize> = qualified
                        .iter()
                        .filter(|&&v_idx| config.volunteers[v_idx].availability_slots[slot_idx])
                        .copied()
                        .collect();
                    let pool = if !available.is_empty() {
                        &available
                    } else {
                        &qualified
                    };
                    schedule.assignments[idx].volunteer_indices = pick_volunteers_biased_internal(
                        config,
                        pool,
                        req,
                        &vol_counts,
                        params.fairness_mode,
                        rng,
                    );
                }
            }
            Mutation::Volunteers { idx, old_vols }
        }
        _ => {
            // Swap two activities' cells. Exchanging cells keeps total occupancy
            // constant only when durations match; with mixed durations the new
            // placements might collide, so we remove both and re-check each cell is
            // free (sequentially, to catch same-field swaps) before committing.
            let idx2 = pick_biased_index(schedule, evaluator, rng, 0.5);
            if idx == idx2 {
                return noop(schedule);
            }

            let s1 = schedule.assignments[idx].slot_idx;
            let f1 = schedule.assignments[idx].field_idx;
            let s2 = schedule.assignments[idx2].slot_idx;
            let f2 = schedule.assignments[idx2].field_idx;
            let dc1 = config.activities[idx].duration_class;
            let dc2 = config.activities[idx2].duration_class;

            let (Some(f1u), Some(f2u)) = (f1, f2) else {
                return noop(schedule);
            };
            // Each activity must be allowed on the other's cell (kind/division/day).
            if !ctx.grid.activity_can_use(config, idx, f2u, s2)
                || !ctx.grid.activity_can_use(config, idx2, f1u, s1)
            {
                return noop(schedule);
            }

            occ.remove(config, f1u, s1, dc1);
            occ.remove(config, f2u, s2, dc2);
            // idx -> (s2,f2): free? then place and check idx2 -> (s1,f1).
            if occ.is_free(config, f2u, s2, dc1) {
                occ.place(config, f2u, s2, dc1);
                if occ.is_free(config, f1u, s1, dc2) {
                    occ.place(config, f1u, s1, dc2);
                    schedule.assignments[idx].slot_idx = s2;
                    schedule.assignments[idx].field_idx = f2;
                    schedule.assignments[idx2].slot_idx = s1;
                    schedule.assignments[idx2].field_idx = f1;
                    return Mutation::Swap {
                        idx1: idx,
                        old_s1: s1,
                        old_f1: f1,
                        idx2,
                        old_s2: s2,
                        old_f2: f2,
                    };
                }
                occ.remove(config, f2u, s2, dc1); // undo tentative place
            }
            // Rollback to the original placement.
            occ.place(config, f1u, s1, dc1);
            occ.place(config, f2u, s2, dc2);
            noop(schedule)
        }
    }
}

/// Reverts a rejected move, restoring both the schedule and the live occupancy.
fn revert_mutation(
    config: &InternalTournamentConfig,
    schedule: &mut InternalSchedule,
    occ: &mut FieldOccupancy,
    mutation: Mutation,
) {
    match mutation {
        Mutation::Relocate {
            idx,
            old_slot,
            old_field,
        } => {
            let cur_slot = schedule.assignments[idx].slot_idx;
            let cur_field = schedule.assignments[idx].field_idx;
            if cur_slot == old_slot && cur_field == old_field {
                return;
            }
            let dc = config.activities[idx].duration_class;
            if let Some(f) = cur_field {
                occ.remove(config, f, cur_slot, dc);
            }
            schedule.assignments[idx].slot_idx = old_slot;
            schedule.assignments[idx].field_idx = old_field;
            if let Some(f) = old_field {
                occ.place(config, f, old_slot, dc);
            }
        }
        Mutation::Volunteers { idx, old_vols } => {
            schedule.assignments[idx].volunteer_indices = old_vols;
        }
        Mutation::Swap {
            idx1,
            old_s1,
            old_f1,
            idx2,
            old_s2,
            old_f2,
        } => {
            let dc1 = config.activities[idx1].duration_class;
            let dc2 = config.activities[idx2].duration_class;
            // Remove both current placements before restoring, so a same-field
            // swap doesn't transiently double-count a bucket.
            let (c1s, c1f) = (
                schedule.assignments[idx1].slot_idx,
                schedule.assignments[idx1].field_idx,
            );
            let (c2s, c2f) = (
                schedule.assignments[idx2].slot_idx,
                schedule.assignments[idx2].field_idx,
            );
            if let Some(f) = c1f {
                occ.remove(config, f, c1s, dc1);
            }
            if let Some(f) = c2f {
                occ.remove(config, f, c2s, dc2);
            }
            schedule.assignments[idx1].slot_idx = old_s1;
            schedule.assignments[idx1].field_idx = old_f1;
            schedule.assignments[idx2].slot_idx = old_s2;
            schedule.assignments[idx2].field_idx = old_f2;
            if let Some(f) = old_f1 {
                occ.place(config, f, old_s1, dc1);
            }
            if let Some(f) = old_f2 {
                occ.place(config, f, old_s2, dc2);
            }
        }
    }
}

fn decompile_schedule(
    config: &TournamentConfig,
    internal_config: &InternalTournamentConfig,
    activities: &[Activity],
    internal: InternalSchedule,
) -> Schedule {
    let assignments = internal
        .assignments
        .into_iter()
        .enumerate()
        .map(|(i, a)| ScheduleAssignment {
            activity: activities[i].clone(),
            time_slot_id: internal_config.slots[a.slot_idx].id.clone(),
            field_id: a.field_idx.map(|f_idx| config.fields[f_idx].id.clone()),
            volunteer_ids: a
                .volunteer_indices
                .iter()
                .map(|&v_idx| config.volunteers[v_idx].id.clone())
                .collect(),
        })
        .collect();
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
            id: "d1".into(),
            name: "Div 1".into(),
            mode: SchedulingMode::HeadToHead,
            games_per_team: 2,
            volunteers_required: 0,
            duration_minutes: 20,
            allowed_fields: None,
            interviews_enabled: false,
            interview_volunteers_required: 0,
            interview_duration_minutes: 0,
            finals_enabled: false,
            finals_rounds: None,
            finals_duration_minutes: None,
            finals_third_place_playoff: false,
            color: None,
            min_match_break_minutes: None,
        });
        for t in ["A", "B", "C", "D"] {
            config.teams.push(Team {
                name: t.into(),
                division_id: "d1".into(),
                organization: t.into(),
            });
        }
        config.fields.push(Field {
            id: "f1".into(),
            name: "Field 1".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        });
        config.fields.push(Field {
            id: "f2".into(),
            name: "Field 2".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        });
        // 10 competition slots, 09:00 .. in 20-minute steps.
        config.time_slots = (0..10)
            .map(|i| slot(&format!("s{i}"), 9 * 60 + i * 20))
            .collect();
        config.day_configs.push(DayGenConfig {
            day: "Saturday".into(),
            ..Default::default()
        });
        config
    }

    #[test]
    fn solves_small_tournament_without_hard_conflicts() {
        let config = small_config();
        let activities = super::super::generate_activities(&config);
        assert!(!activities.is_empty());

        let params = SolverParams {
            max_iterations: 30_000,
            num_restarts: 3,
            ..SolverParams::default()
        };
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
        let base = SolverParams {
            max_iterations: 2_000,
            num_restarts: 2,
            ..SolverParams::default()
        };
        let s1 = solve_schedule(
            &config,
            &SolverParams {
                seed: Some(1),
                ..base.clone()
            },
            |_, _, _, _, _, _| {},
        );
        let s2 = solve_schedule(
            &config,
            &SolverParams {
                seed: Some(2),
                ..base
            },
            |_, _, _, _, _, _| {},
        );
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
            id: id.into(),
            name: id.into(),
            mode,
            games_per_team: games,
            volunteers_required: 1,
            duration_minutes: 20,
            allowed_fields: None,
            interviews_enabled: interviews,
            interview_volunteers_required: 1,
            interview_duration_minutes: 10,
            finals_enabled: false,
            finals_rounds: None,
            finals_duration_minutes: None,
            finals_third_place_playoff: false,
            color: None,
            min_match_break_minutes: None,
        };
        config.divisions = vec![
            div("d1", SchedulingMode::HeadToHead, 3, true),
            div("d2", SchedulingMode::HeadToHead, 2, false),
            div("d3", SchedulingMode::IndividualRun, 2, false),
        ];
        for (d, names) in [
            ("d1", &["A", "B", "C", "D"][..]),
            ("d2", &["E", "F", "G", "H"][..]),
            ("d3", &["I", "J", "K", "L"][..]),
        ] {
            for t in names {
                config.teams.push(Team {
                    name: (*t).into(),
                    division_id: d.into(),
                    organization: format!("org{t}"),
                });
            }
        }
        config.fields = vec![
            Field {
                id: "c1".into(),
                name: "Court 1".into(),
                kind: FieldKind::Competition,
                allowed_divisions: None,
            },
            Field {
                id: "c2".into(),
                name: "Court 2".into(),
                kind: FieldKind::Competition,
                allowed_divisions: Some(vec!["d1".into()]),
            },
            Field {
                id: "c3".into(),
                name: "Court 3".into(),
                kind: FieldKind::Competition,
                allowed_divisions: None,
            },
            Field {
                id: "iv".into(),
                name: "Interview".into(),
                kind: FieldKind::Interview,
                allowed_divisions: None,
            },
        ];
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        let mut slots = Vec::new();
        for (di, day) in ["Saturday", "Sunday"].iter().enumerate() {
            for i in 0..10u32 {
                let start = 9 * 60 + i * 20;
                let kind = if i % 5 == 4 {
                    FieldKind::Interview
                } else {
                    FieldKind::Competition
                };
                slots.push(TimeSlot {
                    id: format!("{day}_s{i}"),
                    day: (*day).into(),
                    start_time: fmt(start),
                    end_time: fmt(start + 20),
                    kind,
                });
            }
            config.day_configs.push(DayGenConfig {
                day: (*day).into(),
                ..Default::default()
            });
            let _ = di;
        }
        config.time_slots = slots;
        config.volunteers = vec![
            Volunteer {
                id: "v1".into(),
                name: "V1".into(),
                availabilities: vec![],
                capabilities: None,
                conflict_organizations: vec![],
                attendance_status: Default::default(),
                locked_field_ids: Some(vec!["c1".into()]),
            },
            Volunteer {
                id: "v2".into(),
                name: "V2".into(),
                availabilities: vec!["Saturday_s0".into(), "Saturday_s1".into()],
                capabilities: Some(vec!["d1".into()]),
                conflict_organizations: vec!["orgA".into()],
                attendance_status: Default::default(),
                locked_field_ids: None,
            },
            Volunteer {
                id: "v3".into(),
                name: "V3".into(),
                availabilities: vec![],
                capabilities: Some(vec!["d2".into(), "d3".into()]),
                conflict_organizations: vec!["orgE".into()],
                attendance_status: Default::default(),
                locked_field_ids: None,
            },
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
        use super::super::fast_evaluator::FastEvaluator;
        use crate::model::SpecialistMode;
        use rand::{Rng, SeedableRng, rngs::StdRng};

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
        // Drive the real move operators from a deliberately messy random start, so
        // the incremental/full agreement is exercised over disorderly states too.
        let mut schedule = construct_initial_internal_schedule(&ic, params.fairness_mode, &mut rng);
        let ctx = MoveCtx::build(&ic);
        let mut occ = super::super::cells::FieldOccupancy::from_schedule(&ic, &schedule);
        let mut ev = FastEvaluator::new(&ic, &params);
        ev.inc_init(&schedule);

        let close =
            |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        let full = |sched: &InternalSchedule| {
            let mut e = FastEvaluator::new(&ic, &params);
            e.calculate_total_cost(sched)
        };

        for step in 0..5000 {
            let inc = ev.inc_total();
            let f = full(&schedule);
            assert!(
                close(inc, f),
                "step {step} pre-mutate: inc {inc:?} != full {f:?}"
            );

            let mutation = mutate_internal_schedule_in_place(
                &ic,
                &ctx,
                &mut schedule,
                &mut occ,
                &mut ev,
                &params,
                &mut rng,
            );
            let olds = mutation.old_assignments(&schedule);
            let inc_after = ev.apply_change(&schedule, &olds);
            let f_after = full(&schedule);
            assert!(
                close(inc_after, f_after),
                "step {step} post-apply: inc {inc_after:?} != full {f_after:?}"
            );

            if rng.gen_bool(0.5) {
                let news: Vec<(usize, InternalAssignment)> = mutation
                    .touched()
                    .into_iter()
                    .map(|i| (i, schedule.assignments[i].clone()))
                    .collect();
                revert_mutation(&ic, &mut schedule, &mut occ, mutation);
                let inc_rev = ev.apply_change(&schedule, &news);
                let f_rev = full(&schedule);
                assert!(
                    close(inc_rev, f_rev),
                    "step {step} post-revert: inc {inc_rev:?} != full {f_rev:?}"
                );
            }
        }
    }

    /// The constructive seed must be valid by construction: every activity placed
    /// on a field, zero field double-bookings (the occupancy gate), and zero
    /// per-team round-order violations (the round-banded layout). These are the
    /// invariants the move set must then preserve.
    #[test]
    fn seed_is_conflict_free_by_construction() {
        use super::super::cells::FieldOccupancy;
        use super::super::conflicts::ConflictKind;
        use super::super::fast_evaluator::FastEvaluator;
        use rand::{SeedableRng, rngs::StdRng};

        let config = guardrail_config();
        let activities = super::super::generate_activities(&config);
        let ic = super::super::internal::InternalTournamentConfig::compile(&config, &activities);
        let params = SolverParams {
            team_match_min_break_minutes: 20,
            ..SolverParams::default()
        };

        let mut rng = StdRng::seed_from_u64(0x5EED5);
        let seed = construct_seed_schedule(&ic, params.fairness_mode, &mut rng);

        // Every activity placed exactly once, each on some field.
        assert_eq!(seed.assignments.len(), activities.len());
        assert!(
            seed.assignments.iter().all(|a| a.field_idx.is_some()),
            "every activity gets a field"
        );

        // Field occupancy gate => no field double-booking.
        assert!(
            !FieldOccupancy::from_schedule(&ic, &seed).has_overlap(),
            "seed must have no field overlap"
        );

        // No FieldDoubleBooked / TeamRoundOrder in the evaluator either.
        let mut ev = FastEvaluator::new(&ic, &params);
        let conflicts = ev.collect_conflicts(&seed);
        let field_db = conflicts
            .iter()
            .filter(|c| matches!(c.kind, ConflictKind::FieldDoubleBooked { .. }))
            .count();
        let round_order = conflicts
            .iter()
            .filter(|c| matches!(c.kind, ConflictKind::TeamRoundOrder { .. }))
            .count();
        assert_eq!(field_db, 0, "seed must have zero field double-bookings");
        assert_eq!(
            round_order, 0,
            "seed must have zero per-team round-order violations"
        );
    }

    /// The structural guarantee of the cell model: starting from the clean seed,
    /// no sequence of moves ever creates a field double-booking, and the live
    /// occupancy stays exactly in sync with the schedule (place/remove balanced
    /// across apply and revert).
    #[test]
    fn moves_never_create_field_overlap() {
        use super::super::cells::FieldOccupancy;
        use super::super::fast_evaluator::FastEvaluator;
        use rand::{Rng, SeedableRng, rngs::StdRng};

        let config = guardrail_config();
        let activities = super::super::generate_activities(&config);
        let ic = super::super::internal::InternalTournamentConfig::compile(&config, &activities);
        let params = SolverParams {
            team_match_min_break_minutes: 20,
            ..SolverParams::default()
        };

        let mut rng = StdRng::seed_from_u64(0xA11CE);
        let mut schedule = construct_seed_schedule(&ic, params.fairness_mode, &mut rng);
        let ctx = MoveCtx::build(&ic);
        let mut occ = FieldOccupancy::from_schedule(&ic, &schedule);
        let mut ev = FastEvaluator::new(&ic, &params);
        ev.inc_init(&schedule);
        assert!(!occ.has_overlap(), "seed must start overlap-free");

        for step in 0..3000 {
            let mutation = mutate_internal_schedule_in_place(
                &ic,
                &ctx,
                &mut schedule,
                &mut occ,
                &mut ev,
                &params,
                &mut rng,
            );
            let olds = mutation.old_assignments(&schedule);
            ev.apply_change(&schedule, &olds);

            // Half the time, reject and revert (the SA path that must also keep occ
            // consistent).
            if rng.gen_bool(0.5) {
                let news: Vec<(usize, InternalAssignment)> = mutation
                    .touched()
                    .into_iter()
                    .map(|i| (i, schedule.assignments[i].clone()))
                    .collect();
                revert_mutation(&ic, &mut schedule, &mut occ, mutation);
                ev.apply_change(&schedule, &news);
            }

            assert!(
                !occ.has_overlap(),
                "step {step}: a move created a field double-booking"
            );
            assert!(
                occ.same_as(&FieldOccupancy::from_schedule(&ic, &schedule)),
                "step {step}: live occupancy drifted from the schedule"
            );
        }
    }

    #[test]
    fn empty_config_yields_empty_schedule() {
        let config = TournamentConfig::default();
        let params = SolverParams::default();
        let schedule =
            solve_schedule(&config, &params, |_, _, _, _, _, _| {}).expect("some schedule");
        assert!(schedule.assignments.is_empty());
    }
}
