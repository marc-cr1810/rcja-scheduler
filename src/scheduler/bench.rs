//! On-demand solver benchmark harness.
//!
//! This is *not* a correctness test — it's a measurement tool. It builds a few
//! representative tournament instances (easy / tight / over-constrained), runs
//! the solver on each several times, and prints the distribution of the final
//! `(hard, soft)` scores, the feasibility rate (fraction of runs reaching zero
//! hard conflicts), the mean time-to-first-feasible, and the mean wall time.
//!
//! It exists so that any change to the solver can be judged against a baseline
//! instead of by eye. Capture the numbers before a change, apply the change,
//! re-run, and compare.
//!
//! The runs are **seeded** (each of the `RUNS_PER_CASE` runs gets a distinct but
//! fixed seed derived from [`BASE_SEED`]), so the whole sweep is deterministic:
//! a difference between two runs reflects the code change, not RNG luck. You
//! still get a distribution across the seeds, just a reproducible one.
//!
//! Run it (release is essential — debug is ~20× slower and not representative):
//!
//! ```text
//! cargo bench-solver                 # alias for the line below (.cargo/config.toml)
//! cargo test --release solver_benchmark -- --ignored --nocapture
//! ```

use super::{SolverParams, evaluate_schedule_cost, solve_schedule};
use crate::model::{
    DayGenConfig, Division, Field, FieldKind, SchedulingMode, Team, TimeSlot, TournamentConfig,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Number of independent solver runs per instance. Each run uses a different
/// fixed seed (`BASE_SEED + run_index`), so we still report a distribution but
/// one that reproduces exactly on the next invocation.
const RUNS_PER_CASE: usize = 5;

/// Base seed for the benchmark. Run `i` of each case uses `BASE_SEED + i`, so
/// the sweep is deterministic across invocations while each run still explores a
/// distinct random stream. Change this to resample the instances.
const BASE_SEED: u64 = 0x5EED;

fn hhmm(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Generates competition (and optionally interview) slots for one day across
/// `[start, end)` minutes, mirroring how the app's auto-generator lays them out.
#[allow(clippy::too_many_arguments)]
fn day_slots(
    day: &str,
    start: u32,
    end: u32,
    comp_dur: u32,
    comp_break: u32,
    int_dur: u32,
    int_break: u32,
    interviews: bool,
) -> Vec<TimeSlot> {
    let mut slots = Vec::new();

    let mut t = start;
    let mut i = 0;
    while t + comp_dur <= end {
        slots.push(TimeSlot {
            id: format!("{day}_c{i}"),
            day: day.into(),
            start_time: hhmm(t),
            end_time: hhmm(t + comp_dur),
            kind: FieldKind::Competition,
        });
        t += comp_dur + comp_break;
        i += 1;
    }

    if interviews {
        let mut t = start;
        let mut j = 0;
        while t + int_dur <= end {
            slots.push(TimeSlot {
                id: format!("{day}_i{j}"),
                day: day.into(),
                start_time: hhmm(t),
                end_time: hhmm(t + int_dur),
                kind: FieldKind::Interview,
            });
            t += int_dur + int_break;
            j += 1;
        }
    }

    slots
}

/// Appends a head-to-head division plus its teams (no volunteers required, finals
/// off — keeps the instance focused on placement, spacing, and breaks).
fn add_division(
    config: &mut TournamentConfig,
    id: &str,
    n_teams: usize,
    games: usize,
    interviews: bool,
) {
    config.divisions.push(Division {
        id: id.into(),
        name: id.into(),
        mode: SchedulingMode::HeadToHead,
        games_per_team: games,
        volunteers_required: 0,
        duration_minutes: 20,
        allowed_fields: None,
        interviews_enabled: interviews,
        interview_volunteers_required: 0,
        interview_duration_minutes: 10,
        finals_enabled: false,
        finals_rounds: None,
        finals_duration_minutes: None,
        finals_third_place_playoff: false,
        color: None,
        min_match_break_minutes: None,
    });
    for k in 0..n_teams {
        config.teams.push(Team {
            name: format!("{id}_T{k}"),
            division_id: id.into(),
            organization: format!("{id}_org{k}"),
        });
    }
}

fn with_day(mut config: TournamentConfig, slots: Vec<TimeSlot>) -> TournamentConfig {
    config.day_configs.push(DayGenConfig {
        day: "Saturday".into(),
        ..Default::default()
    });
    config.time_slots = slots;
    config
}

/// Comfortable: one division, plenty of fields and a long day.
fn easy_case() -> TournamentConfig {
    let mut config = TournamentConfig::default();
    add_division(&mut config, "Soccer", 6, 3, true);
    config.fields = vec![
        Field {
            id: "f1".into(),
            name: "Field 1".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "f2".into(),
            name: "Field 2".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "f3".into(),
            name: "Field 3".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "t1".into(),
            name: "Table 1".into(),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        },
    ];
    // 09:00–17:00.
    with_day(
        config,
        day_slots("Saturday", 9 * 60, 17 * 60, 20, 5, 10, 5, true),
    )
}

/// Tight: enough matches/interviews that the default break floors genuinely bind.
fn tight_case() -> TournamentConfig {
    let mut config = TournamentConfig::default();
    add_division(&mut config, "Soccer", 8, 3, true);
    config.fields = vec![
        Field {
            id: "f1".into(),
            name: "Field 1".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "f2".into(),
            name: "Field 2".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "t1".into(),
            name: "Table 1".into(),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        },
    ];
    // 09:00–13:00 — capacity is close to demand, so spacing is hard to satisfy.
    with_day(
        config,
        day_slots("Saturday", 9 * 60, 13 * 60, 20, 5, 10, 5, true),
    )
}

/// Break-stress: ample raw capacity, but the slot cadence is only 25 min (20-min
/// matches + 5-min gaps), so two adjacent slots are 5 min apart — below the 10-min
/// recharge floor. The instance is feasible, but ONLY if the solver actively
/// spreads each team's matches across non-adjacent slots. This is the case where
/// break handling actually binds, so it's the one a "better" method must improve.
fn break_stress_case() -> TournamentConfig {
    let mut config = TournamentConfig::default();
    add_division(&mut config, "Soccer", 6, 4, true);
    config.fields = vec![
        Field {
            id: "f1".into(),
            name: "Field 1".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "f2".into(),
            name: "Field 2".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "t1".into(),
            name: "Table 1".into(),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        },
    ];
    with_day(
        config,
        day_slots("Saturday", 9 * 60, 15 * 60, 20, 5, 10, 5, true),
    )
}

/// Over-constrained: far more matches than the single field + short day can hold,
/// so zero hard conflicts is unreachable. Measures how *low* a method drives it.
fn over_constrained_case() -> TournamentConfig {
    let mut config = TournamentConfig::default();
    add_division(&mut config, "Soccer", 10, 4, true);
    config.fields = vec![
        Field {
            id: "f1".into(),
            name: "Field 1".into(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        },
        Field {
            id: "t1".into(),
            name: "Table 1".into(),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        },
    ];
    // 09:00–12:00 on one field.
    with_day(
        config,
        day_slots("Saturday", 9 * 60, 12 * 60, 20, 5, 10, 5, true),
    )
}

/// Large: four divisions over two days on many fields. ~140 activities — big
/// enough that the per-iteration evaluation cost dominates, which is exactly
/// where incremental (delta) evaluation is meant to pay off. Feasible, but the
/// solver does real work to place everything.
fn large_case() -> TournamentConfig {
    let mut config = TournamentConfig::default();
    for d in 0..4 {
        add_division(&mut config, &format!("Div{d}"), 12, 4, true);
    }
    config.fields = (0..8)
        .map(|i| Field {
            id: format!("f{i}"),
            name: format!("Field {i}"),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        })
        .chain((0..3).map(|i| Field {
            id: format!("t{i}"),
            name: format!("Table {i}"),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        }))
        .collect();
    let mut slots = day_slots("Saturday", 9 * 60, 17 * 60, 20, 5, 10, 5, true);
    slots.extend(day_slots("Sunday", 9 * 60, 17 * 60, 20, 5, 10, 5, true));
    config.day_configs.push(DayGenConfig {
        day: "Saturday".into(),
        ..Default::default()
    });
    config.day_configs.push(DayGenConfig {
        day: "Sunday".into(),
        ..Default::default()
    });
    config.time_slots = slots;
    config
}

struct RunResult {
    hard: f64,
    soft: f64,
    total_ms: u128,
    /// Wall time (ms) at which a run first reached zero hard conflicts, if ever.
    feasible_ms: Option<u128>,
}

fn run_once(config: &TournamentConfig, params: &SolverParams) -> RunResult {
    let start = Instant::now();
    // Captured from the progress callback (called from worker threads), so use an
    // atomic min: the earliest moment any restart reported zero hard conflicts.
    let feasible_at = Arc::new(AtomicU64::new(u64::MAX));
    let fa = feasible_at.clone();

    let schedule = solve_schedule(config, params, move |_r, _tr, _it, _ti, hard, _soft| {
        if hard == 0.0 {
            let ms = start.elapsed().as_millis() as u64;
            fa.fetch_min(ms, Ordering::Relaxed);
        }
    })
    .expect("solver returned a schedule");

    let total_ms = start.elapsed().as_millis();
    let (hard, soft) = evaluate_schedule_cost(config, &schedule, params);
    let feasible_ms = match feasible_at.load(Ordering::Relaxed) {
        u64::MAX => None,
        v => Some(v as u128),
    };

    RunResult {
        hard,
        soft,
        total_ms,
        feasible_ms,
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn report(name: &str, config: &TournamentConfig, results: &[RunResult]) {
    let activities = super::generate_activities(config).len();
    let comp_slots = config
        .time_slots
        .iter()
        .filter(|s| s.kind == FieldKind::Competition)
        .count();
    let int_slots = config
        .time_slots
        .iter()
        .filter(|s| s.kind == FieldKind::Interview)
        .count();
    let comp_fields = config
        .fields
        .iter()
        .filter(|f| f.kind == FieldKind::Competition)
        .count();
    let int_fields = config
        .fields
        .iter()
        .filter(|f| f.kind == FieldKind::Interview)
        .count();

    let hard: Vec<f64> = results.iter().map(|r| r.hard).collect();
    let soft: Vec<f64> = results.iter().map(|r| r.soft).collect();
    let total: Vec<f64> = results.iter().map(|r| r.total_ms as f64).collect();
    let feasible_runs: Vec<f64> = results
        .iter()
        .filter_map(|r| r.feasible_ms.map(|m| m as f64))
        .collect();

    let hmin = hard.iter().cloned().fold(f64::INFINITY, f64::min);
    let hmax = hard.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let smin = soft.iter().cloned().fold(f64::INFINITY, f64::min);
    let smax = soft.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let feas_rate = feasible_runs.len() as f64 / results.len() as f64 * 100.0;

    println!("\n── {name} ─────────────────────────────────────────");
    println!(
        "  instance: {activities} activities | comp {comp_slots} slots × {comp_fields} fields | interview {int_slots} slots × {int_fields} tables"
    );
    println!("  runs: {}", results.len());
    println!(
        "  HARD  min {hmin:>6.0}  mean {:>8.1}  max {hmax:>6.0}",
        mean(&hard)
    );
    println!(
        "  SOFT  min {smin:>6.1}  mean {:>8.1}  max {smax:>6.1}",
        mean(&soft)
    );
    println!("  feasible (0 hard): {feas_rate:.0}% of runs");
    if !feasible_runs.is_empty() {
        println!(
            "  time-to-feasible:  mean {:>6.0} ms (over feasible runs)",
            mean(&feasible_runs)
        );
    }
    println!("  total solve time:  mean {:>6.0} ms", mean(&total));
}

#[test]
#[ignore = "benchmark; run explicitly with --release --ignored --nocapture"]
fn solver_benchmark() {
    // Match the GUI defaults so the baseline reflects real usage. Kept modest so
    // the whole sweep finishes in a few seconds in release.
    let params = SolverParams {
        max_iterations: 20_000,
        num_restarts: 4,
        ..SolverParams::default()
    };

    println!("\n=== SOLVER BENCHMARK (baseline) ===");
    println!(
        "params: max_iterations={}, restarts={}, runs/case={RUNS_PER_CASE}, base_seed={BASE_SEED:#x} (deterministic)",
        params.max_iterations, params.num_restarts
    );

    for (name, config) in [
        ("EASY", easy_case()),
        ("TIGHT", tight_case()),
        ("BREAK-STRESS", break_stress_case()),
        ("OVER-CONSTRAINED", over_constrained_case()),
        ("LARGE", large_case()),
    ] {
        let results: Vec<RunResult> = (0..RUNS_PER_CASE)
            .map(|i| {
                // Distinct fixed seed per run: reproducible, but still a spread.
                let p = SolverParams {
                    seed: Some(BASE_SEED.wrapping_add(i as u64)),
                    ..params.clone()
                };
                run_once(&config, &p)
            })
            .collect();
        report(name, &config, &results);
    }
    println!();
}
