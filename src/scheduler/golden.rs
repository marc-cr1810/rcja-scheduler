//! Phase 0 — solver re-architecture safety net.
//!
//! This module pins the *real* tournament config (the one behind `schedule.csv`)
//! as a regression fixture and gives the refactor a baseline to move against. It
//! has three jobs:
//!
//!  1. **Feature coverage** — fast, always-on tests asserting the fixture still
//!     exercises every feature the solver must support (multiple divisions,
//!     finals with 3rd-place playoffs, mixed interview/no-interview divisions,
//!     volunteer capabilities / conflicts, division-restricted fields, two days).
//!     If the fixture ever stops covering a feature, these fail and we know the
//!     safety net has a hole *before* trusting it through a phase.
//!
//!  2. **Structural invariants** — properties that must hold for ANY solver
//!     output regardless of quality: every activity placed exactly once, on a
//!     field of the right kind, in a slot of the right kind. These are the
//!     non-negotiables the cell model must never break.
//!
//!  3. **Baseline metrics** — the `golden_baseline` benchmark (ignored; run in
//!     release) solves the real config with a fixed seed and prints the distinct
//!     hard-conflict count broken down by kind, the soft cost, and a dispersion
//!     metric. Capture these before a phase, re-run after, and confirm hard
//!     conflicts only go down and dispersion only improves.
//!
//! Run the baseline with:
//! ```text
//! cargo golden          # alias in .cargo/config.toml
//! ```

#![cfg(test)]

use super::conflicts::{ConflictKind, distinct_hard_conflicts};
use super::{SolverParams, generate_activities};
use crate::model::{Activity, FieldKind, Schedule, TournamentConfig};
use std::collections::BTreeMap;

/// The real tournament config, embedded at compile time so the harness is
/// portable and CWD-independent (the old scratch test hard-coded an absolute
/// path and wrote artifacts into the repo).
const REAL_CONFIG_JSON: &str = include_str!("../../rcja_config.json");

/// Parses the embedded real config.
pub(crate) fn real_config() -> TournamentConfig {
    serde_json::from_str(REAL_CONFIG_JSON).expect("rcja_config.json parses")
}

/// A stable, human-readable name for each conflict kind, used to bucket the
/// baseline report and to assert per-kind counts in later phases.
fn kind_name(k: &ConflictKind) -> &'static str {
    match k {
        ConflictKind::SlotKindMismatch => "SlotKindMismatch",
        ConflictKind::FieldUnsuitable { .. } => "FieldUnsuitable",
        ConflictKind::FieldMissing => "FieldMissing",
        ConflictKind::VolUnavailable { .. } => "VolUnavailable",
        ConflictKind::VolUnqualified { .. } => "VolUnqualified",
        ConflictKind::ConflictOfInterest { .. } => "ConflictOfInterest",
        ConflictKind::VolFieldLocked { .. } => "VolFieldLocked",
        ConflictKind::UnderRostered { .. } => "UnderRostered",
        ConflictKind::InterviewsDisabled => "InterviewsDisabled",
        ConflictKind::DurationExceedsDay => "DurationExceedsDay",
        ConflictKind::DailyShiftCapExceeded { .. } => "DailyShiftCapExceeded",
        ConflictKind::TeamDoubleBooked { .. } => "TeamDoubleBooked",
        ConflictKind::FieldDoubleBooked { .. } => "FieldDoubleBooked",
        ConflictKind::VolDoubleBooked { .. } => "VolDoubleBooked",
        ConflictKind::StageOrder => "StageOrder",
        ConflictKind::StageOverlap => "StageOverlap",
        ConflictKind::FieldVarietyStrict { .. } => "FieldVarietyStrict",
        ConflictKind::TeamMinBreak { .. } => "TeamMinBreak",
        ConflictKind::TeamMatchBreak { .. } => "TeamMatchBreak",
        ConflictKind::TeamRoundOrder { .. } => "TeamRoundOrder",
        ConflictKind::VolCapabilitySoft { .. } => "VolCapabilitySoft",
        ConflictKind::InterviewLate => "InterviewLate",
        ConflictKind::TeamWaitTime => "TeamWaitTime",
        ConflictKind::TeamBackToBack => "TeamBackToBack",
        ConflictKind::InterviewMatchGap => "InterviewMatchGap",
        ConflictKind::VolConsecutive => "VolConsecutive",
        ConflictKind::VolTravel => "VolTravel",
        ConflictKind::RoundOrder => "RoundOrder",
        ConflictKind::FieldVariety => "FieldVariety",
        ConflictKind::FieldBalance => "FieldBalance",
        ConflictKind::VolFairness => "VolFairness",
        ConflictKind::Specialist => "Specialist",
        ConflictKind::PeakPeriod => "PeakPeriod",
    }
}

/// Distinct hard conflicts in `schedule`, grouped by kind name. This is the
/// canonical per-kind breakdown the refactor drives toward zero. Built from the
/// same structured records the GUI and solver use, so it can never disagree with
/// the headline count.
pub(crate) fn hard_conflicts_by_kind(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> BTreeMap<&'static str, usize> {
    let (_internal, records, dropped) =
        super::fast_evaluator::evaluate_schedule_conflicts(config, schedule, params);
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for c in distinct_hard_conflicts(&records) {
        *counts.entry(kind_name(&c.kind)).or_default() += 1;
    }
    if !dropped.is_empty() {
        *counts.entry("DroppedActivity").or_default() += dropped.len();
    }
    counts
}

/// Total distinct hard conflicts (sum over kinds).
#[allow(dead_code)] // used by later-phase regression tests
pub(crate) fn hard_conflict_total(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> usize {
    hard_conflicts_by_kind(config, schedule, params)
        .values()
        .sum()
}

/// Round-robin dispersion — the coefficient of variation (std / mean) of the
/// number of **non-final** competition activities per `(day, start-time)` band,
/// measured over the chronological bands **before the finals block**.
///
/// That pre-finals region is the time RR is meant to fill evenly across both
/// days; the finals tail legitimately holds little/no RR, so including it would
/// just punish the (desired) reserved finals block. Lower is more even; 0.0 is a
/// perfectly flat RR schedule over its region.
pub(crate) fn dispersion(config: &TournamentConfig, schedule: &Schedule) -> f64 {
    let fp = finals_profile(config, schedule);
    // RR fills every band up to where the finals block begins (or all bands if
    // there are no finals).
    let cutoff = fp.first_finals_band.unwrap_or(fp.total_comp_bands);
    if cutoff == 0 {
        return 0.0;
    }

    // Chronological comp band index for each slot id (mirrors finals_profile).
    let mut day_order: BTreeMap<String, usize> = BTreeMap::new();
    for dc in &config.day_configs {
        let d = dc.day.to_lowercase();
        if !day_order.contains_key(&d) {
            let i = day_order.len();
            day_order.insert(d, i);
        }
    }
    for s in &config.time_slots {
        let d = s.day.to_lowercase();
        if !day_order.contains_key(&d) {
            let i = day_order.len();
            day_order.insert(d, i);
        }
    }
    let band_key = |day: &str, start: &str| {
        (
            day_order.get(&day.to_lowercase()).copied().unwrap_or(99),
            start.to_string(),
        )
    };
    let mut comp_bands: Vec<(usize, String)> = config
        .time_slots
        .iter()
        .filter(|s| s.kind == FieldKind::Competition)
        .map(|s| band_key(&s.day, &s.start_time))
        .collect();
    comp_bands.sort();
    comp_bands.dedup();
    let band_index: BTreeMap<(usize, String), usize> = comp_bands
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let slot_band: BTreeMap<&str, usize> = config
        .time_slots
        .iter()
        .filter_map(|s| {
            band_index
                .get(&band_key(&s.day, &s.start_time))
                .map(|&i| (s.id.as_str(), i))
        })
        .collect();

    let mut counts = vec![0.0f64; cutoff];
    for a in &schedule.assignments {
        if matches!(a.activity, Activity::Interview { .. }) || a.activity.is_final() {
            continue;
        }
        if let Some(&bi) = slot_band.get(a.time_slot_id.as_str())
            && bi < cutoff
        {
            counts[bi] += 1.0;
        }
    }

    let n = counts.len() as f64;
    let mean = counts.iter().sum::<f64>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = counts.iter().map(|&c| (c - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() / mean
}

/// How tightly and how late the finals sit. Competition bands are ordered
/// chronologically (day, then start); the finals should occupy a short run of
/// bands at the very end.
pub(crate) struct FinalsProfile {
    pub total_comp_bands: usize,
    /// Chronological band index of the first / last band containing any final.
    pub first_finals_band: Option<usize>,
    pub last_finals_band: Option<usize>,
    /// Distinct bands that contain at least one final.
    pub finals_band_count: usize,
    /// Round-robin matches scheduled at or after the first finals band — ideally
    /// small (RR should mostly precede the finals block).
    pub rr_at_or_after_finals: usize,
}

pub(crate) fn finals_profile(config: &TournamentConfig, schedule: &Schedule) -> FinalsProfile {
    // Day order: config.day_configs first, then first-seen in time_slots.
    let mut day_order: BTreeMap<String, usize> = BTreeMap::new();
    for dc in &config.day_configs {
        let d = dc.day.to_lowercase();
        if !day_order.contains_key(&d) {
            let i = day_order.len();
            day_order.insert(d, i);
        }
    }
    for s in &config.time_slots {
        let d = s.day.to_lowercase();
        if !day_order.contains_key(&d) {
            let i = day_order.len();
            day_order.insert(d, i);
        }
    }
    let band_key = |day: &str, start: &str| {
        (
            day_order.get(&day.to_lowercase()).copied().unwrap_or(99),
            start.to_string(),
        )
    };

    // Chronological competition bands.
    let mut comp_bands: Vec<(usize, String)> = config
        .time_slots
        .iter()
        .filter(|s| s.kind == FieldKind::Competition)
        .map(|s| band_key(&s.day, &s.start_time))
        .collect();
    comp_bands.sort();
    comp_bands.dedup();
    let band_index: BTreeMap<(usize, String), usize> = comp_bands
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();

    let slot_key: BTreeMap<&str, (usize, String)> = config
        .time_slots
        .iter()
        .map(|s| (s.id.as_str(), band_key(&s.day, &s.start_time)))
        .collect();

    let mut first = None;
    let mut last = None;
    let mut finals_bands: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut rr_at_or_after = 0;
    // First pass: find earliest finals band.
    for a in &schedule.assignments {
        if a.activity.is_final()
            && let Some(k) = slot_key.get(a.time_slot_id.as_str())
            && let Some(&bi) = band_index.get(k)
        {
            finals_bands.insert(bi);
            first = Some(first.map_or(bi, |f: usize| f.min(bi)));
            last = Some(last.map_or(bi, |l: usize| l.max(bi)));
        }
    }
    if let Some(f) = first {
        for a in &schedule.assignments {
            if matches!(a.activity, Activity::Interview { .. }) || a.activity.is_final() {
                continue;
            }
            if let Some(k) = slot_key.get(a.time_slot_id.as_str())
                && let Some(&bi) = band_index.get(k)
                && bi >= f
            {
                rr_at_or_after += 1;
            }
        }
    }

    FinalsProfile {
        total_comp_bands: comp_bands.len(),
        first_finals_band: first,
        last_finals_band: last,
        finals_band_count: finals_bands.len(),
        rr_at_or_after_finals: rr_at_or_after,
    }
}

/// Builds [`SolverParams`] from a config's stored `solver_settings`, mirroring
/// `AppState::get_solver_params` so the baseline reflects real usage. `seed`,
/// `iterations`, and `restarts` are overridable to keep the harness fast and
/// deterministic.
pub(crate) fn params_from_config(
    config: &TournamentConfig,
    seed: u64,
    iterations: usize,
    restarts: usize,
) -> SolverParams {
    let s = &config.solver_settings;
    SolverParams {
        max_iterations: iterations,
        num_restarts: restarts,
        fairness_mode: s.fairness_mode,
        vol_consecutive_weight: s.vol_consecutive_weight,
        team_back_to_back_weight: s.team_back_to_back_weight,
        field_variety_weight: s.field_variety_weight,
        field_balance_weight: s.field_balance_weight,
        vol_capability_weight: s.vol_capability_weight,
        interview_late_weight: s.interview_late_weight,
        interview_match_gap_weight: s.interview_match_gap_weight,
        team_min_break_minutes: s.team_min_break_minutes,
        team_break_buffer_minutes: s.team_break_buffer_minutes,
        team_match_min_break_minutes: s.team_match_min_break_minutes,
        team_match_break_buffer_minutes: s.team_match_break_buffer_minutes,
        vol_specialist_mode: s.vol_specialist_mode,
        team_wait_time_weight: s.team_wait_time_weight,
        field_variety_strict: s.field_variety_strict,
        vol_travel_weight: s.vol_travel_weight,
        round_order_weight: s.round_order_weight,
        vol_daily_shift_cap: s.vol_daily_shift_cap,
        peak_period_weight: s.peak_period_weight,
        finals_priority_multiplier: s.finals_priority_multiplier,
        cancel_flag: None,
        seed: Some(seed),
    }
}

#[cfg(test)]
mod tests {
    use super::super::solve_schedule;
    use super::*;
    use crate::model::{Activity, FieldKind, SchedulingMode};

    /// Fixed seed + the real config's own iteration budget (50k×5) so the
    /// baseline reflects what the user actually sees, deterministically.
    ///
    /// History at this seed: the pre-rewrite solver landed ~88 distinct hard
    /// conflicts (≈86 `TeamRoundOrder` + a couple of `FieldDoubleBooked`), soft
    /// ≈2222, dispersion CoV ≈0.255. Phase 2's constructive seeder dropped that to
    /// **0 hard conflicts**, soft ≈937, CoV ≈0.234; Phase 3's free-relocation move
    /// set took CoV to ≈0.198; Phase 5 (banding removed, `peak_period_weight`
    /// raised to 1.0) reached 0 hard, soft ≈941. Phase 7 (global RR spread + finals
    /// excluded from the spread metric + seeder reserves a stage-banded finals tail
    /// placed most-constrained-division-first) reaches **0 hard, soft ≈950, RR-CoV
    /// ≈0.17, finals in the last ~4 bands reaching the final slot** (RR-CoV is over
    /// the pre-finals region — see `dispersion`). Field-poor divisions may keep a
    /// couple of QFs the band before the block — unavoidable when a division's QF
    /// count exceeds its field count and the post-lunch tail is short.
    const GOLDEN_SEED: u64 = 0x60_1DEC0DE;
    const GOLDEN_ITERS: usize = 50_000;
    const GOLDEN_RESTARTS: usize = 5;

    /// Baseline metrics for the real config. Ignored by default (the full solve
    /// is too slow for the always-on suite and must run in release); invoke with
    /// `cargo golden`. Asserts the invariants that must ALWAYS hold and prints the
    /// hard-conflict breakdown + dispersion so later phases can confirm progress.
    #[test]
    #[ignore = "baseline benchmark; run with: cargo golden"]
    fn golden_baseline() {
        let config = real_config();
        let params = params_from_config(&config, GOLDEN_SEED, GOLDEN_ITERS, GOLDEN_RESTARTS);
        let activities = generate_activities(&config);

        let start = std::time::Instant::now();
        let schedule = solve_schedule(&config, &params, |_, _, _, _, _, _| {}).expect("solved");
        let ms = start.elapsed().as_millis();

        // --- Structural invariants (must hold for ANY solver output) ---
        assert_eq!(
            schedule.assignments.len(),
            activities.len(),
            "every activity must be placed exactly once"
        );
        for a in &schedule.assignments {
            let field_id = a.field_id.as_ref().expect("every activity gets a field");
            let field = config
                .fields
                .iter()
                .find(|f| &f.id == field_id)
                .expect("field exists");
            let slot = config
                .time_slots
                .iter()
                .find(|s| s.id == a.time_slot_id)
                .expect("slot exists");
            let is_interview = matches!(a.activity, Activity::Interview { .. });
            let want = if is_interview {
                FieldKind::Interview
            } else {
                FieldKind::Competition
            };
            assert_eq!(
                field.kind, want,
                "activity placed on wrong field kind: {:?}",
                a.activity
            );
            assert_eq!(
                slot.kind, want,
                "activity placed in wrong slot kind: {:?}",
                a.activity
            );
        }

        // --- Baseline metrics ---
        let by_kind = hard_conflicts_by_kind(&config, &schedule, &params);
        let total: usize = by_kind.values().sum();
        let (hard, soft) = super::super::evaluate_schedule_cost(&config, &schedule, &params);
        let disp = dispersion(&config, &schedule);

        println!("\n=== GOLDEN BASELINE (real config) ===");
        println!(
            "  seed={GOLDEN_SEED:#x} iters={GOLDEN_ITERS} restarts={GOLDEN_RESTARTS} | {} activities | solve {ms} ms",
            activities.len()
        );
        println!("  HARD distinct conflicts: {total}  (cost hard={hard})");
        for (k, n) in &by_kind {
            println!("    {k:<22} {n}");
        }
        println!("  SOFT cost: {soft:.1}");
        println!("  RR DISPERSION (CoV, lower=more even): {disp:.4}");
        let fp = finals_profile(&config, &schedule);
        println!(
            "  FINALS: bands {}..={} of {} total | {} bands wide | {} RR matches at/after finals start",
            fp.first_finals_band.map_or("-".into(), |v| v.to_string()),
            fp.last_finals_band.map_or("-".into(), |v| v.to_string()),
            fp.total_comp_bands.saturating_sub(1),
            fp.finals_band_count,
            fp.rr_at_or_after_finals,
        );
        println!("=====================================\n");

        // Regression lock: as of Phase 2 the real config solves to ZERO hard
        // conflicts at this seed. Keep it there — Phase 4 makes field overlap
        // structurally impossible, and no phase may regress feasibility. Soft
        // cost / dispersion are optimised separately (Phase 5).
        assert_eq!(
            total, 0,
            "hard-conflict regression: expected 0, got {total} (breakdown: {by_kind:?})"
        );
    }

    /// The embedded fixture must parse and still exercise the full feature set
    /// the solver supports. If a future config edit drops a feature, this fails
    /// so we don't trust an under-powered safety net.
    #[test]
    fn fixture_covers_all_features() {
        let c = real_config();

        assert!(c.divisions.len() >= 2, "needs multiple divisions");
        assert!(c.teams.len() >= 20, "needs a realistic team count");

        // Finals (with 3rd-place playoff) must be present.
        assert!(
            c.divisions
                .iter()
                .any(|d| d.finals_enabled && d.finals_third_place_playoff),
            "fixture must cover finals + 3rd-place playoff"
        );
        // Mixed interview / no-interview divisions.
        assert!(
            c.divisions.iter().any(|d| d.interviews_enabled),
            "needs an interview division"
        );
        assert!(
            c.divisions.iter().any(|d| !d.interviews_enabled),
            "needs a no-interview division"
        );

        // Field kinds: both competition fields and interview tables.
        assert!(c.fields.iter().any(|f| f.kind == FieldKind::Competition));
        assert!(c.fields.iter().any(|f| f.kind == FieldKind::Interview));
        // Division-restricted fields exercise the allowed_divisions path.
        assert!(
            c.fields.iter().any(|f| f.allowed_divisions.is_some()),
            "needs a restricted field"
        );

        // Volunteer constraint surfaces.
        assert!(
            c.volunteers.iter().any(|v| v.capabilities.is_some()),
            "needs capability limits"
        );
        assert!(
            c.volunteers
                .iter()
                .any(|v| !v.conflict_organizations.is_empty()),
            "needs a conflict of interest"
        );

        // Two days, both with slots.
        assert!(c.day_configs.len() >= 2, "needs a multi-day event");
        let days: std::collections::BTreeSet<&str> =
            c.time_slots.iter().map(|s| s.day.as_str()).collect();
        assert!(days.len() >= 2, "slots must span both days");

        // All four divisions head-to-head here; just confirm the mode resolves.
        assert!(c.divisions.iter().all(|d| matches!(
            d.mode,
            SchedulingMode::HeadToHead | SchedulingMode::IndividualRun
        )));
    }

    /// The problem compiles and generates a sane activity set: finals matches,
    /// interviews, and round-robin matches all appear.
    #[test]
    fn fixture_generates_expected_activities() {
        let c = real_config();
        let acts = generate_activities(&c);
        assert!(!acts.is_empty());

        let finals = acts.iter().filter(|a| a.is_final()).count();
        let interviews = acts
            .iter()
            .filter(|a| matches!(a, Activity::Interview { .. }))
            .count();
        let rr = acts
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    Activity::Match {
                        stage: crate::model::MatchStage::RoundRobin { .. },
                        ..
                    }
                )
            })
            .count();

        assert!(finals > 0, "expected finals matches");
        assert!(interviews > 0, "expected interview activities");
        assert!(rr > 0, "expected round-robin matches");

        // Compiling the internal config must not panic and must keep every activity.
        let ic = super::super::internal::InternalTournamentConfig::compile(&c, &acts);
        assert_eq!(ic.activities.len(), acts.len());
    }

    /// Dispersion metric sanity: a perfectly even spread scores ~0, a clustered
    /// spread scores higher. Guards the metric itself before we optimise against it.
    #[test]
    fn dispersion_metric_rewards_even_spread() {
        use crate::model::{Field, MatchStage, ScheduleAssignment, TimeSlot};
        let mut c = TournamentConfig::default();
        c.fields = vec![
            Field {
                id: "f1".into(),
                name: "F1".into(),
                kind: FieldKind::Competition,
                allowed_divisions: None,
            },
            Field {
                id: "f2".into(),
                name: "F2".into(),
                kind: FieldKind::Competition,
                allowed_divisions: None,
            },
        ];
        // Four competition bands at 09:00, 09:20, 09:40, 10:00.
        c.time_slots = (0..4)
            .map(|i| {
                let m = 9 * 60 + i * 20;
                TimeSlot {
                    id: format!("s{i}"),
                    day: "Sat".into(),
                    start_time: format!("{:02}:{:02}", m / 60, m % 60),
                    end_time: format!("{:02}:{:02}", (m + 20) / 60, (m + 20) % 60),
                    kind: FieldKind::Competition,
                }
            })
            .collect();

        let mk = |slot: &str| ScheduleAssignment {
            activity: Activity::Match {
                id: slot.into(),
                team_a: "a".into(),
                team_b: "b".into(),
                division_id: "d".into(),
                duration_minutes: 20,
                stage: MatchStage::RoundRobin { cycle: 0, round: 0 },
            },
            time_slot_id: slot.into(),
            field_id: Some("f1".into()),
            volunteer_ids: vec![],
        };

        // Even: one activity in each of the four bands.
        let even = Schedule {
            assignments: (0..4).map(|i| mk(&format!("s{i}"))).collect(),
        };
        // Clustered: all four in the first band, last three empty.
        let clustered = Schedule {
            assignments: (0..4).map(|_| mk("s0")).collect(),
        };

        let d_even = dispersion(&c, &even);
        let d_clustered = dispersion(&c, &clustered);
        assert!(d_even < 1e-9, "even spread should score ~0, got {d_even}");
        assert!(d_clustered > d_even, "clustered must score worse than even");
    }
}
