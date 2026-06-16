//! Phase 1 of the solver re-architecture: the `(field × time)` **cell grid**.
//!
//! The old model treats `slot_idx` and `field_idx` as independent free variables,
//! so two activities can land on the same field at overlapping times and field
//! double-booking is only discouraged by a penalty. The cell model replaces that
//! with a resource the search can't over-fill:
//!
//!  * A [`Cell`] is a concrete placement position — field `f` starting at the time
//!    of slot `s`. The catalog ([`CellGrid`]) holds every `(field, slot)` whose
//!    kinds match (competition fields host competition slots; interview tables
//!    host interview slots).
//!  * [`FieldOccupancy`] is the **gate**: a placement is valid only if the field
//!    is free for the activity's whole `[start, start + duration)` span. Because a
//!    long activity spills into later slots' time buckets, the gate — not the
//!    one-cell-per-activity rule alone — is what makes field overlap impossible to
//!    express. It mirrors the evaluator's `activity_buckets` scheme, so an
//!    occupancy with no bucket above 1 is exactly a schedule with zero
//!    `FieldDoubleBooked` conflicts.
//!
//! This module is **additive**: nothing wires it into the solver yet. Phase 2
//! (constructive seeder) and Phase 3 (relocate / swap moves) consume it; Phase 4
//! removes the now-redundant field-double-booking penalty.
//!
//! The seeder and move set consume most of this; a few small accessors
//! (`cell_index`, `candidate_cells`, `FieldOccupancy::same_as`) are public cell-
//! model API currently exercised only by the unit tests, hence the module allow.
#![allow(dead_code)]

use super::internal::InternalTournamentConfig;

/// A candidate placement position: field `field_idx` at the start time of
/// `slot_idx`. An activity placed here occupies the field for
/// `[slot.start, slot.start + activity.duration)`, which may span later slots
/// when the activity is longer than the slot-grid step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub field_idx: usize,
    pub slot_idx: usize,
}

/// Static catalog of every kind-matching `(field, slot)` cell, built once from a
/// compiled config. The catalog is division-agnostic; whether a *given* activity
/// may occupy a cell is decided per-activity by [`CellGrid::activity_can_use`]
/// (field-division match, interviews-enabled-that-day, fits-in-day).
pub struct CellGrid {
    pub cells: Vec<Cell>,
    /// Cell index by `field_idx * num_slots + slot_idx`; `usize::MAX` where the
    /// field and slot kinds don't match (no cell exists there).
    lookup: Vec<usize>,
    num_slots: usize,
    /// Per-day end minute (max slot end on that day), indexed by `day_idx`. Used
    /// to reject placements whose duration would run past the end of the day —
    /// the same rule the evaluator enforces as `DurationExceedsDay`.
    day_end: Vec<u32>,
}

impl CellGrid {
    pub fn build(config: &InternalTournamentConfig) -> Self {
        let num_fields = config.fields.len();
        let num_slots = config.slots.len();

        let num_days = config.slots.iter().map(|s| s.day_idx + 1).max().unwrap_or(0);
        let mut day_end = vec![0u32; num_days];
        for s in &config.slots {
            let end = s.start_minutes + s.duration_minutes;
            if end > day_end[s.day_idx] {
                day_end[s.day_idx] = end;
            }
        }

        let mut cells = Vec::new();
        let mut lookup = vec![usize::MAX; num_fields * num_slots];
        for (field_idx, field) in config.fields.iter().enumerate() {
            for (slot_idx, slot) in config.slots.iter().enumerate() {
                if field.kind == slot.kind {
                    lookup[field_idx * num_slots + slot_idx] = cells.len();
                    cells.push(Cell { field_idx, slot_idx });
                }
            }
        }

        Self { cells, lookup, num_slots, day_end }
    }

    /// The cell index for `(field, slot)`, or `None` if the kinds don't match.
    pub fn cell_index(&self, field_idx: usize, slot_idx: usize) -> Option<usize> {
        let i = self.lookup.get(field_idx * self.num_slots + slot_idx).copied()?;
        if i == usize::MAX { None } else { Some(i) }
    }

    /// Whether `activity` may be hard-feasibly placed at `(field, slot)` ignoring
    /// occupancy: kinds match, the field allows the division, interviews are
    /// enabled that day, and the activity fits before end-of-day. Round-window /
    /// ordering preferences are the seeder's concern, not a usability gate.
    pub fn activity_can_use(
        &self,
        config: &InternalTournamentConfig,
        activity_idx: usize,
        field_idx: usize,
        slot_idx: usize,
    ) -> bool {
        let act = &config.activities[activity_idx];
        let field = &config.fields[field_idx];
        let slot = &config.slots[slot_idx];

        let want = if act.is_interview { crate::model::FieldKind::Interview } else { crate::model::FieldKind::Competition };
        if field.kind != want || slot.kind != want {
            return false;
        }
        if let Some(allowed) = &field.allowed_division_indices
            && !allowed.contains(&act.division_idx)
        {
            return false;
        }
        if act.is_interview && !config.day_interviews_enabled[slot.day_idx] {
            return false;
        }
        if slot.start_minutes + act.duration_minutes > self.day_end[slot.day_idx] {
            return false;
        }
        true
    }

    /// Iterator over cell indices `activity` may hard-feasibly use (ignoring
    /// occupancy). Phase 2 intersects this with the round window and free cells.
    pub fn candidate_cells<'a>(
        &'a self,
        config: &'a InternalTournamentConfig,
        activity_idx: usize,
    ) -> impl Iterator<Item = usize> + 'a {
        self.cells.iter().enumerate().filter_map(move |(ci, c)| {
            if self.activity_can_use(config, activity_idx, c.field_idx, c.slot_idx) {
                Some(ci)
            } else {
                None
            }
        })
    }
}

/// Per-field time-bucket occupancy: the validity gate behind the cell model.
///
/// Buckets are the same 5-minute global buckets the evaluator uses
/// (`config.activity_buckets[slot][duration_class]`), so a placement is valid iff
/// every bucket it would touch on that field is currently empty. Maintained as an
/// invariant by the seeder/moves, every count stays `<= 1`, which is precisely
/// "no field double-booking" expressed structurally rather than as a penalty.
pub struct FieldOccupancy {
    /// `[field_idx][bucket]` occupancy count.
    occ: Vec<Vec<u32>>,
}

impl FieldOccupancy {
    pub fn new(num_fields: usize, num_buckets: usize) -> Self {
        Self { occ: vec![vec![0; num_buckets]; num_fields] }
    }

    /// Whether `field_idx` is free for an activity of `duration_class` starting at
    /// `slot_idx` — i.e. every bucket the placement touches is empty.
    pub fn is_free(
        &self,
        config: &InternalTournamentConfig,
        field_idx: usize,
        slot_idx: usize,
        duration_class: usize,
    ) -> bool {
        config.activity_buckets[slot_idx][duration_class]
            .iter()
            .all(|&b| self.occ[field_idx][b] == 0)
    }

    /// Marks the field busy for the placement's span. Caller must have checked
    /// [`Self::is_free`] to keep the no-overlap invariant.
    pub fn place(
        &mut self,
        config: &InternalTournamentConfig,
        field_idx: usize,
        slot_idx: usize,
        duration_class: usize,
    ) {
        for &b in &config.activity_buckets[slot_idx][duration_class] {
            self.occ[field_idx][b] += 1;
        }
    }

    /// Frees the field over the placement's span. Inverse of [`Self::place`].
    pub fn remove(
        &mut self,
        config: &InternalTournamentConfig,
        field_idx: usize,
        slot_idx: usize,
        duration_class: usize,
    ) {
        for &b in &config.activity_buckets[slot_idx][duration_class] {
            self.occ[field_idx][b] = self.occ[field_idx][b].saturating_sub(1);
        }
    }

    /// Builds occupancy from an existing schedule (every activity with a field).
    pub fn from_schedule(
        config: &InternalTournamentConfig,
        schedule: &super::internal::InternalSchedule,
    ) -> Self {
        let mut o = Self::new(config.fields.len(), config.num_total_buckets);
        for (i, a) in schedule.assignments.iter().enumerate() {
            if let Some(f) = a.field_idx {
                o.place(config, f, a.slot_idx, config.activities[i].duration_class);
            }
        }
        o
    }

    /// Whether any field bucket holds more than one activity — i.e. the schedule
    /// has at least one field double-booking. Zero here is exactly the property
    /// the cell model guarantees by construction.
    pub fn has_overlap(&self) -> bool {
        self.occ.iter().any(|row| row.iter().any(|&c| c > 1))
    }

    /// Whether two occupancies hold identical counts. Used by tests to prove the
    /// incrementally-maintained occupancy never drifts from a rebuild.
    pub fn same_as(&self, other: &FieldOccupancy) -> bool {
        self.occ == other.occ
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::scheduler::internal::{InternalAssignment, InternalSchedule, InternalTournamentConfig};

    /// Two competition fields (one restricted to d1), one interview table; one
    /// short day with competition + interview slots; two divisions so the
    /// allowed-division gate is exercised.
    fn test_config() -> TournamentConfig {
        let mut c = TournamentConfig::default();
        let div = |id: &str, dur: u32, interviews: bool| Division {
            id: id.into(), name: id.into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 2, volunteers_required: 0, duration_minutes: dur,
            allowed_fields: None, interviews_enabled: interviews,
            interview_volunteers_required: 0, interview_duration_minutes: 10,
            finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
            finals_third_place_playoff: false, color: None, min_match_break_minutes: None,
        };
        c.divisions = vec![div("d1", 20, true), div("d2", 20, false)];
        for (d, names) in [("d1", ["A", "B", "C", "D"]), ("d2", ["E", "F", "G", "H"])] {
            for t in names {
                c.teams.push(Team { name: t.into(), division_id: d.into(), organization: format!("org{t}") });
            }
        }
        c.fields = vec![
            Field { id: "f1".into(), name: "F1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "F2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["d1".into()]) },
            Field { id: "t1".into(), name: "T1".into(), kind: FieldKind::Interview, allowed_divisions: None },
        ];
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        // 09:00..11:00, 20-min comp slots; interview slots interleaved.
        let mut slots = Vec::new();
        for i in 0..6u32 {
            let m = 9 * 60 + i * 20;
            slots.push(TimeSlot { id: format!("c{i}"), day: "Sat".into(), start_time: fmt(m), end_time: fmt(m + 20), kind: FieldKind::Competition });
            slots.push(TimeSlot { id: format!("it{i}"), day: "Sat".into(), start_time: fmt(m), end_time: fmt(m + 10), kind: FieldKind::Interview });
        }
        c.time_slots = slots;
        c.day_configs.push(DayGenConfig { day: "Sat".into(), interviews_enabled: true, ..Default::default() });
        c
    }

    fn compile(c: &TournamentConfig) -> (InternalTournamentConfig, Vec<Activity>) {
        let acts = crate::scheduler::generate_activities(c);
        let ic = InternalTournamentConfig::compile(c, &acts);
        (ic, acts)
    }

    #[test]
    fn grid_has_one_cell_per_kind_matching_field_slot() {
        let c = test_config();
        let (ic, _) = compile(&c);
        let grid = CellGrid::build(&ic);

        let comp_fields = ic.fields.iter().filter(|f| f.kind == FieldKind::Competition).count();
        let int_fields = ic.fields.iter().filter(|f| f.kind == FieldKind::Interview).count();
        let comp_slots = ic.slots.iter().filter(|s| s.kind == FieldKind::Competition).count();
        let int_slots = ic.slots.iter().filter(|s| s.kind == FieldKind::Interview).count();

        assert_eq!(grid.cells.len(), comp_fields * comp_slots + int_fields * int_slots);

        // Every cell round-trips through the lookup, and is kind-consistent.
        for (ci, cell) in grid.cells.iter().enumerate() {
            assert_eq!(grid.cell_index(cell.field_idx, cell.slot_idx), Some(ci));
            assert_eq!(ic.fields[cell.field_idx].kind, ic.slots[cell.slot_idx].kind);
        }
    }

    #[test]
    fn candidacy_respects_kind_division_and_day() {
        let c = test_config();
        let (ic, acts) = compile(&c);
        let grid = CellGrid::build(&ic);

        // A d2 match (no interviews) must never be usable on an interview field/slot,
        // nor on the d1-restricted competition field f2.
        let d2_idx = ic.divisions.iter().position(|d| d.id == "d2").unwrap();
        let f2 = ic.fields.iter().position(|f| f.id == "f2").unwrap();
        let d2_match = acts.iter().position(|a| !matches!(a, Activity::Interview { .. }) && {
            let ai = acts.iter().position(|x| x.id() == a.id()).unwrap();
            ic.activities[ai].division_idx == d2_idx
        }).unwrap();

        // Restricted field rejects d2.
        let comp_slot = ic.slots.iter().position(|s| s.kind == FieldKind::Competition).unwrap();
        assert!(!grid.activity_can_use(&ic, d2_match, f2, comp_slot), "d2 must not use d1-only field");

        // Interview slot/field rejects a match.
        let int_field = ic.fields.iter().position(|f| f.kind == FieldKind::Interview).unwrap();
        let int_slot = ic.slots.iter().position(|s| s.kind == FieldKind::Interview).unwrap();
        assert!(!grid.activity_can_use(&ic, d2_match, int_field, int_slot), "match must not use interview cell");

        // An unrestricted competition field accepts it.
        let f1 = ic.fields.iter().position(|f| f.id == "f1").unwrap();
        assert!(grid.activity_can_use(&ic, d2_match, f1, comp_slot));

        // Every candidate cell genuinely passes the gate.
        for ci in grid.candidate_cells(&ic, d2_match) {
            let cell = grid.cells[ci];
            assert!(grid.activity_can_use(&ic, d2_match, cell.field_idx, cell.slot_idx));
        }
    }

    #[test]
    fn occupancy_gate_blocks_same_field_overlap_only() {
        let c = test_config();
        let (ic, _) = compile(&c);
        let f1 = ic.fields.iter().position(|f| f.id == "f1").unwrap();
        let f2 = ic.fields.iter().position(|f| f.id == "f2").unwrap();
        // First two competition slots (distinct start times).
        let comp: Vec<usize> = ic.slots.iter().enumerate().filter(|(_, s)| s.kind == FieldKind::Competition).map(|(i, _)| i).collect();
        let (s0, s1) = (comp[0], comp[1]);
        let dc = ic.activities[0].duration_class; // a 20-min match's class

        let mut occ = FieldOccupancy::new(ic.fields.len(), ic.num_total_buckets);
        assert!(occ.is_free(&ic, f1, s0, dc));
        occ.place(&ic, f1, s0, dc);

        // Same field + same slot → blocked. Same time, different field → free.
        assert!(!occ.is_free(&ic, f1, s0, dc));
        assert!(occ.is_free(&ic, f2, s0, dc));
        // Same field, a different (non-overlapping) slot → free.
        assert!(occ.is_free(&ic, f1, s1, dc));

        // Removing restores freedom and leaves no overlap behind.
        occ.remove(&ic, f1, s0, dc);
        assert!(occ.is_free(&ic, f1, s0, dc));
        assert!(!occ.has_overlap());
    }

    #[test]
    fn has_overlap_matches_double_booking() {
        let c = test_config();
        let (ic, _) = compile(&c);
        let f1 = ic.fields.iter().position(|f| f.id == "f1").unwrap();
        let comp: Vec<usize> = ic.slots.iter().enumerate().filter(|(_, s)| s.kind == FieldKind::Competition).map(|(i, _)| i).collect();

        // Two activities on the same field+slot is an overlap; building occupancy
        // from such a schedule must report it.
        let sched = InternalSchedule {
            assignments: vec![
                InternalAssignment { slot_idx: comp[0], field_idx: Some(f1), volunteer_indices: vec![] },
                InternalAssignment { slot_idx: comp[0], field_idx: Some(f1), volunteer_indices: vec![] },
            ],
        };
        // Note: from_schedule indexes activities by position, so use the first two
        // real activities' duration classes implicitly via index 0/1 — both 20-min.
        let occ = FieldOccupancy::from_schedule(&ic, &sched);
        assert!(occ.has_overlap(), "same field+slot twice must register as overlap");
    }
}
