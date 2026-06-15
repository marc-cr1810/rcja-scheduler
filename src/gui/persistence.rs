use std::fs::{self, File};
use std::io::Write;
use std::collections::HashSet;
use crate::model::{Team, ScheduleAssignment};
use super::{AppState, ExportMessage};
use genpdf::{elements, style, Element};

#[derive(Clone, Copy, PartialEq)]
enum ReportType {
    Master,
    Division,
    Team,
    Volunteer,
}

impl AppState {
    pub fn save_config(&mut self) {
        if let Some(path) = &self.current_file_path {
            self.save_to_path(path.clone());
        } else {
            self.save_config_as();
        }
    }

    pub fn save_config_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_title("Save Configuration As")
            .save_file()
        {
            self.save_to_path(path.clone());
            self.current_file_path = Some(path);
        }
    }

    fn save_to_path(&mut self, path: std::path::PathBuf) {
        // Sync solver/generator settings from AppState into the config before saving
        self.sync_solver_settings_to_config();

        let json = match serde_json::to_string_pretty(&self.config) {
            Ok(json) => json,
            Err(e) => {
                self.status_message = format!("Failed to serialize config: {}", e);
                return;
            }
        };
        if let Ok(mut file) = File::create(&path) {
            if file.write_all(json.as_bytes()).is_ok() {
                self.status_message = format!("Config saved to '{}'", path.display());
            } else {
                self.status_message = "Failed to write file".to_string();
            }
        } else {
            self.status_message = "Failed to create file".to_string();
        }
    }

    pub fn load_config(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_title("Load Configuration")
            .pick_file()
        {
            if let Ok(file) = File::open(&path) {
                if let Ok(config) = serde_json::from_reader(file) {
                    self.config = config;
                    self.current_file_path = Some(path.clone());
                    // Migrate legacy fairness_mode for old configs that lack solver_settings
                    self.migrate_legacy_fairness_mode();
                    // Populate AppState solver fields from the loaded config
                    self.sync_solver_settings_from_config();
                    self.clear_schedule();
                    self.update_diagnostics();
                    self.status_message = format!("Config loaded from '{}'", path.display());
                } else {
                    self.status_message = format!("Failed to parse '{}'", path.display());
                }
            } else {
                self.status_message = format!("'{}' not found", path.display());
            }
        }
    }

    /// Copy solver/generator settings from AppState fields into `self.config.solver_settings`.
    pub fn sync_solver_settings_to_config(&mut self) {
        let ss = &mut self.config.solver_settings;
        ss.fairness_mode = self.solver_fairness_mode;
        ss.iterations = self.solver_iterations;
        ss.restarts = self.solver_restarts;
        ss.use_seed = self.solver_use_seed;
        ss.seed = self.solver_seed;
        ss.vol_consecutive_weight = self.solver_vol_consecutive_weight;
        ss.team_back_to_back_weight = self.solver_team_back_to_back_weight;
        ss.field_variety_weight = self.solver_field_variety_weight;
        ss.field_balance_weight = self.solver_field_balance_weight;
        ss.vol_capability_weight = self.solver_vol_capability_weight;
        ss.interview_late_weight = self.solver_interview_late_weight;
        ss.interview_match_gap_weight = self.solver_interview_match_gap_weight;
        ss.team_min_break_minutes = self.solver_team_min_break_minutes;
        ss.team_break_buffer_minutes = self.solver_team_break_buffer_minutes;
        ss.team_match_min_break_minutes = self.solver_team_match_min_break_minutes;
        ss.team_match_break_buffer_minutes = self.solver_team_match_break_buffer_minutes;
        ss.vol_specialist_mode = self.solver_vol_specialist_mode;
        ss.team_wait_time_weight = self.solver_team_wait_time_weight;
        ss.field_variety_strict = self.solver_field_variety_strict;
        ss.vol_travel_weight = self.solver_vol_travel_weight;
        ss.round_order_weight = self.solver_round_order_weight;
        ss.vol_daily_shift_cap = self.solver_vol_daily_shift_cap;
        ss.peak_period_weight = self.solver_peak_period_weight;
        ss.finals_priority_multiplier = self.solver_finals_priority_multiplier;
        ss.gen_slot_duration = self.gen_slot_duration;
        ss.gen_interview_slot_duration = self.gen_interview_slot_duration;
        ss.gen_match_slot_break = self.gen_match_slot_break;
        ss.gen_interview_slot_break = self.gen_interview_slot_break;
        // Keep legacy field in sync
        self.config.fairness_mode = self.solver_fairness_mode;
    }

    /// Copy solver/generator settings from `self.config.solver_settings` into AppState fields.
    pub fn sync_solver_settings_from_config(&mut self) {
        let ss = &self.config.solver_settings;
        self.solver_fairness_mode = ss.fairness_mode;
        self.solver_iterations = ss.iterations;
        self.solver_restarts = ss.restarts;
        self.solver_use_seed = ss.use_seed;
        self.solver_seed = ss.seed;
        self.solver_vol_consecutive_weight = ss.vol_consecutive_weight;
        self.solver_team_back_to_back_weight = ss.team_back_to_back_weight;
        self.solver_field_variety_weight = ss.field_variety_weight;
        self.solver_field_balance_weight = ss.field_balance_weight;
        self.solver_vol_capability_weight = ss.vol_capability_weight;
        self.solver_interview_late_weight = ss.interview_late_weight;
        self.solver_interview_match_gap_weight = ss.interview_match_gap_weight;
        self.solver_team_min_break_minutes = ss.team_min_break_minutes;
        self.solver_team_break_buffer_minutes = ss.team_break_buffer_minutes;
        self.solver_team_match_min_break_minutes = ss.team_match_min_break_minutes;
        self.solver_team_match_break_buffer_minutes = ss.team_match_break_buffer_minutes;
        self.solver_vol_specialist_mode = ss.vol_specialist_mode;
        self.solver_team_wait_time_weight = ss.team_wait_time_weight;
        self.solver_field_variety_strict = ss.field_variety_strict;
        self.solver_vol_travel_weight = ss.vol_travel_weight;
        self.solver_round_order_weight = ss.round_order_weight;
        self.solver_vol_daily_shift_cap = ss.vol_daily_shift_cap;
        self.solver_peak_period_weight = ss.peak_period_weight;
        self.solver_finals_priority_multiplier = ss.finals_priority_multiplier;
        self.gen_slot_duration = ss.gen_slot_duration;
        self.gen_interview_slot_duration = ss.gen_interview_slot_duration;
        self.gen_match_slot_break = ss.gen_match_slot_break;
        self.gen_interview_slot_break = ss.gen_interview_slot_break;
    }

    /// For old config files that have a top-level `fairness_mode` but no `solver_settings`,
    /// migrate the legacy value into `solver_settings.fairness_mode`.
    fn migrate_legacy_fairness_mode(&mut self) {
        // If solver_settings has the default fairness mode but the legacy field doesn't,
        // it means this is an old config — adopt the legacy value.
        if self.config.solver_settings.fairness_mode == crate::model::FairnessMode::Balanced
            && self.config.fairness_mode != crate::model::FairnessMode::Balanced
        {
            self.config.solver_settings.fairness_mode = self.config.fairness_mode;
        }
    }

    pub fn export_to_csv(&mut self) {
        if let Some(ref schedule) = self.schedule {
            let csv = generate_csv_content_internal(&self.config, &schedule.assignments, self.export_options.time_12h);
            
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_title("Export Schedule As")
                .save_file()
            {
                if let Ok(mut file) = File::create(&path) {
                    if file.write_all(csv.as_bytes()).is_ok() {
                        self.status_message = format!("Schedule exported to '{}'", path.display());
                    } else {
                        self.status_message = "Failed to write CSV".to_string();
                    }
                } else {
                    self.status_message = "Failed to create file".to_string();
                }
            }
        }
    }

    pub fn export_volunteer_rosters_to_csv(&mut self) {
        if let Some(ref schedule) = self.schedule {
            let mut csv = String::from("Volunteer,Time,Day,Field,Activity,Division\n");
            
            let mut volunteers = self.config.volunteers.clone();
            volunteers.sort_by_key(|v| v.name.clone());

            for vol in volunteers {
                let mut vol_assigns: Vec<_> = schedule.assignments.iter()
                    .filter(|a| a.volunteer_ids.contains(&vol.id))
                    .collect();
                
                vol_assigns.sort_by_key(|a| {
                    let slot = self.config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                    slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
                });

                for assign in vol_assigns {
                    let slot = match self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id) {
                        Some(s) => s,
                        None => continue,
                    };
                    let field = assign.field_id.as_ref().and_then(|fid| self.config.fields.iter().find(|f| f.id == *fid));
                    let div = self.config.divisions.iter().find(|d| d.id == assign.activity.division_id());

                    csv.push_str(&format!(
                        "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
                        csv_field(&vol.name), csv_field(&format_time_str(&slot.start_time, self.export_options.time_12h)), csv_field(&slot.day),
                        csv_field(field.map_or("", |f| &f.name)), csv_field(&assign.activity.export_label()),
                        csv_field(div.map_or("", |d| &d.name))
                    ));
                }
            }
            
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_title("Export Volunteer Rosters As")
                .save_file()
            {
                if let Ok(mut file) = File::create(&path) {
                    if file.write_all(csv.as_bytes()).is_ok() {
                        self.status_message = format!("Volunteer rosters exported to '{}'", path.display());
                    } else {
                        self.status_message = "Failed to write CSV".to_string();
                    }
                } else {
                    self.status_message = "Failed to create file".to_string();
                }
            }
        }
    }

    pub fn export_full_tournament(&mut self) {
        if self.schedule.is_none() {
            self.status_message = "No schedule to export!".to_string();
            return;
        }

        if let Some(base_path) = rfd::FileDialog::new()
            .set_title("Select Export Directory")
            .pick_folder()
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.export_rx = Some(rx);
            self.is_exporting = true;
            self.export_progress = 0.0;
            
            let config = self.config.clone();
            let schedule = self.schedule.clone().unwrap();
            let opts = self.export_options;
            let tournament_name = clean_filename(&config.competition_name);
            let root = base_path.join(format!("Export_{}", tournament_name));

            std::thread::spawn(move || {
                if let Err(e) = fs::create_dir_all(&root) {
                    let _ = tx.send(ExportMessage::Error(format!("Failed to create directory: {}", e)));
                    return;
                }

                // Top-level format folders (only those the user asked for).
                let csv_root = root.join("csv");
                let pdf_root = root.join("pdf");
                if opts.csv { let _ = fs::create_dir_all(&csv_root); }
                if opts.pdf { let _ = fs::create_dir_all(&pdf_root); }

                // Subdirectories within format folders, created on demand by the
                // selected report categories.
                let csv_div = csv_root.join("Divisions");
                let csv_team = csv_root.join("Teams");
                let csv_vol = csv_root.join("Volunteers");
                let pdf_div = pdf_root.join("Divisions");
                let pdf_team = pdf_root.join("Teams");
                let pdf_vol = pdf_root.join("Volunteers");
                if opts.divisions {
                    if opts.csv { let _ = fs::create_dir_all(&csv_div); }
                    if opts.pdf { let _ = fs::create_dir_all(&pdf_div); }
                }
                if opts.teams {
                    if opts.csv { let _ = fs::create_dir_all(&csv_team); }
                    if opts.pdf { let _ = fs::create_dir_all(&pdf_team); }
                }
                if opts.volunteers {
                    if opts.csv { let _ = fs::create_dir_all(&csv_vol); }
                    if opts.pdf { let _ = fs::create_dir_all(&pdf_vol); }
                }

                let divisions = config.divisions.clone();
                let teams = config.teams.clone();
                let volunteers = config.volunteers.clone();
                let total_steps = (opts.master as usize)
                    + if opts.divisions { divisions.len() } else { 0 }
                    + if opts.teams { teams.len() } else { 0 }
                    + if opts.volunteers { volunteers.len() } else { 0 };
                let total_steps = total_steps.max(1);
                let mut current_step = 0;

                let writer = ExportWriter {
                    config: config.clone(),
                    pdf: opts.pdf,
                    csv: opts.csv,
                    time_12h: opts.time_12h,
                };

                // 1. Master Schedule
                if opts.master {
                    writer.write_export_files(&csv_root, &pdf_root, "Master_Schedule", &schedule.assignments, ReportType::Master);
                    current_step += 1;
                    let _ = tx.send(ExportMessage::Progress(current_step as f32 / total_steps as f32));
                }

                // 2. Division Schedules
                if opts.divisions {
                    for div in &divisions {
                        let div_assigns: Vec<ScheduleAssignment> = schedule.assignments.iter()
                            .filter(|a| a.activity.division_id() == div.id)
                            .cloned()
                            .collect();
                        if !div_assigns.is_empty() {
                            let div_safe_name = clean_filename(&div.name);
                            writer.write_export_files(&csv_div, &pdf_div, &div_safe_name, &div_assigns, ReportType::Division);
                        }
                        current_step += 1;
                        let _ = tx.send(ExportMessage::Progress(current_step as f32 / total_steps as f32));
                    }
                }

                // 3. Team Schedules
                if opts.teams {
                    for team in &teams {
                        let team_assigns: Vec<ScheduleAssignment> = schedule.assignments.iter()
                            .filter(|a| a.activity.teams().contains(&team.name.as_str()))
                            .cloned()
                            .collect();
                        if !team_assigns.is_empty() {
                            let team_safe_name = clean_filename(&team.name);
                            writer.write_export_files(&csv_team, &pdf_team, &team_safe_name, &team_assigns, ReportType::Team);
                        }
                        current_step += 1;
                        let _ = tx.send(ExportMessage::Progress(current_step as f32 / total_steps as f32));
                    }
                }

                // 4. Volunteer Schedules
                if opts.volunteers {
                    for vol in &volunteers {
                        let vol_assigns: Vec<ScheduleAssignment> = schedule.assignments.iter()
                            .filter(|a| a.volunteer_ids.contains(&vol.id))
                            .cloned()
                            .collect();
                        if !vol_assigns.is_empty() {
                            let vol_safe_name = clean_filename(&vol.name);
                            writer.write_export_files(&csv_vol, &pdf_vol, &vol_safe_name, &vol_assigns, ReportType::Volunteer);
                        }
                        current_step += 1;
                        let _ = tx.send(ExportMessage::Progress(current_step as f32 / total_steps as f32));
                    }
                }

                let _ = tx.send(ExportMessage::Done(format!("Export completed to '{}'", root.display())));
            });
        }
    }
}

struct ExportWriter {
    config: crate::model::TournamentConfig,
    pdf: bool,
    csv: bool,
    time_12h: bool,
}

impl ExportWriter {
    fn write_export_files(&self, csv_dir: &std::path::Path, pdf_dir: &std::path::Path, name: &str, assignments: &[ScheduleAssignment], report_type: ReportType) {
        if self.csv {
            let csv_content = generate_csv_content_internal(&self.config, assignments, self.time_12h);
            let csv_path = csv_dir.join(format!("{}.csv", name));
            let _ = fs::write(csv_path, csv_content);
        }

        if self.pdf {
            let pdf_path = pdf_dir.join(format!("{}.pdf", name));
            match generate_pdf_document_internal(&self.config, name, assignments, report_type, self.time_12h) {
                Some(doc) => {
                    if let Err(e) = doc.render_to_file(&pdf_path) {
                        eprintln!("Failed to render PDF {}: {}", pdf_path.display(), e);
                    }
                }
                None => {
                    eprintln!("Failed to generate PDF document for {}", name);
                }
            }
        }
    }
}

fn generate_csv_content_internal(config: &crate::model::TournamentConfig, assignments: &[ScheduleAssignment], time_12h: bool) -> String {
    let mut csv = String::from("Field,Time,Division,Activity,Team A,Org A,Team B,Org B,Volunteers\n");
    
    let mut sorted_assigns = assignments.to_vec();
    sorted_assigns.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        slot.map(|s| (s.day.clone(), s.start_minutes()))
    });

    for assign in sorted_assigns {
        let slot = config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
        let field = assign.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
        let division = config.divisions.iter().find(|d| d.id == assign.activity.division_id());
        
        let time_str = slot.map_or("".to_string(), |s| format!("{} {}", day_abbr(&s.day), format_time_str(&s.start_time, time_12h)));
        let div_name = division.map_or("", |d| &d.name);
        let field_name = field.map_or("", |f| &f.name);
        
        let (team_a, org_a, team_b, org_b) = match &assign.activity {
            crate::model::Activity::Match { team_a, team_b, .. } => {
                let o_a = config.teams.iter().find(|t| &t.name == team_a).map(|t| t.organization.as_str()).unwrap_or("");
                let o_b = config.teams.iter().find(|t| &t.name == team_b).map(|t| t.organization.as_str()).unwrap_or("");
                (team_a.as_str(), o_a, team_b.as_str(), o_b)
            }
            crate::model::Activity::Run { team, .. } | crate::model::Activity::Interview { team, .. } => {
                let o = config.teams.iter().find(|t| &t.name == team).map(|t| t.organization.as_str()).unwrap_or("");
                (team.as_str(), o, "", "")
            }
        };

        let vols: Vec<String> = assign.volunteer_ids.iter()
            .map(|vid| config.volunteers.iter().find(|v| v.id == *vid).map_or(vid.clone(), |v| v.name.clone()))
            .collect();
        let vol_names = vols.join("; ");

        csv.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            csv_field(field_name), csv_field(&time_str), csv_field(div_name),
            csv_field(&assign.activity.export_label()), csv_field(team_a), csv_field(org_a),
            csv_field(team_b), csv_field(org_b), csv_field(&vol_names)
        ));
    }
    csv
}

fn get_activity_metadata(config: &crate::model::TournamentConfig, activity: &crate::model::Activity) -> String {
    let div_id = activity.division_id();
    let div_name = config.divisions.iter().find(|d| d.id == div_id).map(|d| d.name.as_str()).unwrap_or(div_id);
    
    match activity {
        crate::model::Activity::Match { team_a, team_b, .. } => {
            let org_a = config.teams.iter().find(|t| &t.name == team_a).map(|t| t.organization.as_str()).unwrap_or("");
            let org_b = config.teams.iter().find(|t| &t.name == team_b).map(|t| t.organization.as_str()).unwrap_or("");
            if org_a == org_b && !org_a.is_empty() {
                format!("{} - {}", div_name, org_a)
            } else if !org_a.is_empty() && !org_b.is_empty() {
                format!("{} - {} / {}", div_name, org_a, org_b)
            } else if !org_a.is_empty() {
                format!("{} - {}", div_name, org_a)
            } else if !org_b.is_empty() {
                format!("{} - {}", div_name, org_b)
            } else {
                div_name.to_string()
            }
        }
        crate::model::Activity::Run { team, .. } | crate::model::Activity::Interview { team, .. } => {
            let org = config.teams.iter().find(|t| &t.name == team).map(|t| t.organization.as_str()).unwrap_or("");
            if !org.is_empty() {
                format!("{} - {}", div_name, org)
            } else {
                div_name.to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PDF design system
//
// genpdf 0.2 cannot paint a solid rectangle behind text — cell decorators run
// *after* the cell content is drawn, and only hairline strokes are exposed — so
// the previous "fill the header by drawing 40 stacked hairlines" hack produced
// a stripey block with the text showing through. The report is now built
// entirely from typography, colour and horizontal rules, which renders cleanly
// and identically on every platform.
// ---------------------------------------------------------------------------

// Type scale (pt)
const FS_TITLE: u8 = 20;
const FS_SUBTITLE: u8 = 12;
const FS_SECTION: u8 = 12;
const FS_TH: u8 = 8;
const FS_BODY: u8 = 9;
const FS_META: u8 = 8;
const FS_FOOTER: u8 = 7;

// Palette
const C_PRIMARY: style::Color = style::Color::Rgb(30, 58, 138);
const C_ACCENT: style::Color = style::Color::Rgb(79, 70, 229);
const C_MUTED: style::Color = style::Color::Rgb(100, 116, 139);
const C_RULE: style::Color = style::Color::Rgb(203, 213, 225);
const C_FOOTER: style::Color = style::Color::Rgb(148, 163, 184);

/// Loads the Liberation Sans family that is embedded in the binary at compile
/// time. Bundling the font means PDF export no longer depends on whatever fonts
/// happen to be installed: the old code looked up system fonts by a naming
/// convention (`Arial-Regular.ttf`) that does not exist on Windows, so
/// `from_files` always returned `None` and export silently failed there.
fn load_embedded_font() -> Option<genpdf::fonts::FontFamily<genpdf::fonts::FontData>> {
    use genpdf::fonts::{FontData, FontFamily};
    macro_rules! face {
        ($file:literal) => {
            FontData::new(
                include_bytes!(concat!("../../assets/fonts/", $file)).to_vec(),
                None,
            )
            .ok()?
        };
    }
    Some(FontFamily {
        regular: face!("LiberationSans-Regular.ttf"),
        bold: face!("LiberationSans-Bold.ttf"),
        italic: face!("LiberationSans-Italic.ttf"),
        bold_italic: face!("LiberationSans-BoldItalic.ttf"),
    })
}

/// Draws a horizontal hairline spanning the full width of `area` at height `y`.
fn draw_hline(area: &genpdf::render::Area<'_>, y: genpdf::Mm, color: style::Color) {
    let width = area.size().width;
    area.draw_line(
        vec![
            genpdf::Position { x: 0.into(), y },
            genpdf::Position { x: width, y },
        ],
        style::Style::new().with_color(color),
    );
}

/// A standalone full-width divider usable anywhere an `Element` is accepted (the
/// document body, or nested inside a card). `strong` stacks a second hairline
/// just above the first for a heavier rule.
fn rule_element(color: style::Color, strong: bool) -> elements::TableLayout {
    let mut table = elements::TableLayout::new(vec![1]);
    table.set_cell_decorator(RuleDecorator { color, strong });
    let mut row = table.row();
    row.push_element(elements::Paragraph::new(" ").styled(style::Style::new().with_font_size(1)));
    row.push().ok();
    table
}

struct RuleDecorator {
    color: style::Color,
    strong: bool,
}

impl genpdf::elements::CellDecorator for RuleDecorator {
    fn decorate_cell(&mut self, _column: usize, _row: usize, _has_more: bool, area: genpdf::render::Area<'_>, _style: style::Style) {
        let h = area.size().height;
        draw_hline(&area, h * 0.5, self.color);
        if self.strong {
            draw_hline(&area, h * 0.62, self.color);
        }
    }
}

/// Cell decorator that draws a rule along the bottom edge of every cell. Used
/// for the schedule header (heavy primary rule, `strong`) and the body rows
/// (light rule), giving clean ledger-style separation without filled blocks.
struct BottomRuleDecorator {
    color: style::Color,
    strong: bool,
}

impl genpdf::elements::CellDecorator for BottomRuleDecorator {
    fn decorate_cell(&mut self, _column: usize, _row: usize, _has_more: bool, area: genpdf::render::Area<'_>, _style: style::Style) {
        let h = area.size().height;
        draw_hline(&area, h, self.color);
        if self.strong {
            draw_hline(&area, h * 0.985, self.color);
        }
    }
}

// Column labels and widths for the schedule tables, kept together so the
// repeated header always lines up with the body columns.
const SCHEDULE_COLUMNS: [&str; 5] = ["TIME", "ROUND", "FIELD", "ACTIVITY", "DIVISION"];
const SCHEDULE_WEIGHTS: [usize; 5] = [4, 3, 3, 8, 6];

/// Cell padding (mm) applied to every schedule cell via `.padded(PAD)`. Shared
/// so row-height measurement matches what is actually rendered.
const PAD: u8 = 2;

/// One schedule row: the styled text for each column.
struct ScheduleRow {
    cells: Vec<(String, style::Style)>,
}

/// A schedule table that keeps each row intact across page boundaries and
/// re-prints its column header at the top of every page it spans.
///
/// genpdf 0.2's `TableLayout` draws the header only once and will happily split a
/// tall (word-wrapped) row across a page break, leaving an orphaned fragment.
/// This element instead owns the rows as data: on each `render` call it draws a
/// fresh header, then places rows one at a time, measuring each against the
/// remaining space and stopping before one would overflow. The document re-calls
/// `render` with a new full-page area whenever `has_more` is set, so the header
/// reappears and the remaining rows continue cleanly on the next page.
struct RepeatingScheduleTable {
    title: String,
    rows: Vec<ScheduleRow>,
    next: usize,
    /// Set once the section has been deferred to a fresh page, so we never defer
    /// twice in a row (which could loop forever on a section taller than a page).
    deferred: bool,
}

fn section_title_style() -> style::Style {
    style::Style::new().bold().with_font_size(FS_SECTION).with_color(C_PRIMARY)
}

impl RepeatingScheduleTable {
    fn build_header() -> elements::TableLayout {
        let mut header = elements::TableLayout::new(SCHEDULE_WEIGHTS.to_vec());
        header.set_cell_decorator(BottomRuleDecorator { color: C_PRIMARY, strong: true });
        let th = style::Style::new().bold().with_font_size(FS_TH).with_color(C_PRIMARY);
        let mut row = header.row();
        for label in SCHEDULE_COLUMNS {
            row.push_element(elements::Paragraph::new(label).styled(th).padded(PAD));
        }
        row.push().ok();
        header
    }

    fn build_row(row: &ScheduleRow) -> elements::TableLayout {
        let mut table = elements::TableLayout::new(SCHEDULE_WEIGHTS.to_vec());
        table.set_cell_decorator(BottomRuleDecorator { color: C_RULE, strong: false });
        let mut r = table.row();
        for (text, st) in &row.cells {
            r.push_element(elements::Paragraph::new(text.clone()).styled(*st).padded(PAD));
        }
        r.push().ok();
        table
    }

    /// Conservative height of `row` when laid out at `table_width`, mirroring
    /// genpdf's greedy word wrapping. A one-line safety margin is added so a
    /// slight estimate miss can never cause a mid-row page split.
    fn row_height(context: &genpdf::Context, table_width: genpdf::Mm, row: &ScheduleRow) -> genpdf::Mm {
        let weight_sum: usize = SCHEDULE_WEIGHTS.iter().sum();
        let pad = genpdf::Mm::from(PAD);
        let mut max_h = genpdf::Mm::from(0u8);
        for (i, (text, st)) in row.cells.iter().enumerate() {
            let col_w = table_width * (SCHEDULE_WEIGHTS[i] as f64 / weight_sum as f64);
            let avail = col_w - pad - pad;
            let lines = wrapped_line_count(context, st, text, avail) + 1; // +1 safety
            let h = st.line_height(&context.font_cache) * lines as f64 + pad + pad;
            max_h = max_h.max(h);
        }
        max_h
    }
}

/// Greedy first-fit word-wrap line count for `text` at width `avail`, using the
/// same glyph metrics genpdf uses when it renders the paragraph.
fn wrapped_line_count(context: &genpdf::Context, st: &style::Style, text: &str, avail: genpdf::Mm) -> usize {
    let zero = genpdf::Mm::from(0u8);
    let space = st.str_width(&context.font_cache, " ");
    let mut lines = 1usize;
    let mut cur = zero;
    for word in text.split_whitespace() {
        let w = st.str_width(&context.font_cache, word);
        if cur <= zero {
            cur = w;
        } else if cur + space + w <= avail {
            cur = cur + space + w;
        } else {
            lines += 1;
            cur = w;
        }
    }
    lines
}

impl Element for RepeatingScheduleTable {
    fn render(
        &mut self,
        context: &genpdf::Context,
        mut area: genpdf::render::Area<'_>,
        style: style::Style,
    ) -> Result<genpdf::RenderResult, genpdf::error::Error> {
        let mut result = genpdf::RenderResult::default();
        if self.next >= self.rows.len() {
            return Ok(result);
        }

        // Keep the section title with the column header and first row: if they
        // won't fit together, defer the whole section to a fresh page so the
        // title is never stranded alone at the bottom of the previous page.
        if self.next == 0 {
            let title_h = section_title_style().line_height(&context.font_cache)
                + style.line_height(&context.font_cache) * 0.4 // the 0.4 break below the title
                + genpdf::Mm::from(PAD) * 2.0; // header padding
            let header_h = style::Style::new().bold().with_font_size(FS_TH).line_height(&context.font_cache);
            let first_row_h = Self::row_height(context, area.size().width, &self.rows[0]);
            if !self.deferred && title_h + header_h + first_row_h > area.size().height {
                self.deferred = true;
                result.has_more = true;
                return Ok(result);
            }
            let mut title = elements::Paragraph::new(self.title.clone()).styled(section_title_style());
            let title_result = title.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, title_result.size.height));
            result.size = result.size.stack_vertical(title_result.size);
            let mut brk = elements::Break::new(0.4);
            let brk_result = brk.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, brk_result.size.height));
            result.size = result.size.stack_vertical(brk_result.size);
        }

        let mut header = Self::build_header();
        let header_result = header.render(context, area.clone(), style)?;
        area.add_offset(genpdf::Position::new(0, header_result.size.height));
        result.size = result.size.stack_vertical(header_result.size);

        let table_width = area.size().width;
        let mut placed_any = false;
        while self.next < self.rows.len() {
            let row = &self.rows[self.next];
            let needed = Self::row_height(context, table_width, row);
            // Break to a fresh page before a row would overflow — unless nothing
            // has been placed yet on this page (a row taller than a whole page),
            // in which case render it anyway to guarantee forward progress.
            if needed > area.size().height && placed_any {
                break;
            }

            let row_result = Self::build_row(row).render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, row_result.size.height));
            result.size = result.size.stack_vertical(row_result.size);
            placed_any = true;
            self.next += 1;
        }

        result.has_more = self.next < self.rows.len();
        Ok(result)
    }
}

fn render_schedule_section(doc: &mut genpdf::Document, config: &crate::model::TournamentConfig, section_title: &str, assignments: &[&ScheduleAssignment], time_12h: bool) {
    if assignments.is_empty() { return; }

    // One row per assignment, sorted by day / time / field. Rows are collected
    // as data so RepeatingScheduleTable can keep each row whole across page
    // breaks, repeat the column header on every page, and keep the section title
    // attached to the first row.
    let mut sorted: Vec<&ScheduleAssignment> = assignments.to_vec();
    sorted.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        (
            slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time))),
            a.field_id.clone(),
        )
    });

    let time_style = style::Style::new().bold().with_font_size(FS_BODY);
    let round_style = style::Style::new().with_font_size(FS_BODY).with_color(C_MUTED);
    let field_style = style::Style::new().with_font_size(FS_BODY);
    let activity_style = style::Style::new().bold().with_font_size(FS_BODY);
    let division_style = style::Style::new().with_font_size(FS_META).with_color(C_MUTED);

    let mut rows = Vec::new();
    for assign in sorted {
        let slot = match config.time_slots.iter().find(|s| s.id == assign.time_slot_id) {
            Some(s) => s,
            None => continue,
        };
        let field_name = assign.field_id.as_ref()
            .and_then(|fid| config.fields.iter().find(|f| f.id == *fid))
            .map_or_else(|| "-".to_string(), |f| clean_text(&f.name));

        rows.push(ScheduleRow {
            cells: vec![
                (format!("{} {}", day_abbr(&slot.day), format_time_str(&slot.start_time, time_12h)), time_style),
                (assign.activity.round_label(), round_style),
                (field_name, field_style),
                (clean_text(&assign.activity.export_label()), activity_style),
                (clean_text(&get_activity_metadata(config, &assign.activity)), division_style),
            ],
        });
    }

    doc.push(RepeatingScheduleTable { title: section_title.to_string(), rows, next: 0, deferred: false });
    doc.push(elements::Break::new(1.0));
}

// Column widths for the per-activity rows inside a team card.
const TEAM_WEIGHTS: [usize; 3] = [4, 8, 5];

fn team_name_style() -> style::Style { style::Style::new().bold().with_font_size(FS_BODY + 1).with_color(C_PRIMARY) }
fn team_org_style() -> style::Style { style::Style::new().with_font_size(FS_META).with_color(C_MUTED) }

/// A single team's personal schedule, pre-extracted as plain data so the grid
/// can both measure and build each card during rendering.
struct TeamCard {
    name: String,
    org: String,
    rows: Vec<(String, String, String)>, // (time, detail, field)
}

/// Collects and sorts one team's assignments into a printable [`TeamCard`].
fn prepare_team_card(config: &crate::model::TournamentConfig, team: &Team, assignments: &[ScheduleAssignment], time_12h: bool) -> TeamCard {
    let mut team_assigns: Vec<_> = assignments.iter()
        .filter(|a| a.activity.teams().contains(&team.name.as_str()))
        .collect();
    team_assigns.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
    });

    let mut rows = Vec::new();
    for assign in team_assigns {
        let slot = match config.time_slots.iter().find(|s| s.id == assign.time_slot_id) {
            Some(s) => s,
            None => continue,
        };
        let field_name = assign.field_id.as_ref()
            .and_then(|fid| config.fields.iter().find(|f| f.id == *fid))
            .map_or_else(|| "-".to_string(), |f| clean_text(&f.name));
        let detail = match &assign.activity {
            crate::model::Activity::Match { team_a, team_b, .. } => {
                let opp = if team_a == &team.name { team_b } else { team_a };
                format!("vs {}", clean_text(opp))
            }
            crate::model::Activity::Run { run_number, .. } => format!("Run #{}", run_number),
            crate::model::Activity::Interview { .. } => "Interview".to_string(),
        };
        rows.push((format!("{} {}", day_abbr(&slot.day), format_time_str(&slot.start_time, time_12h)), detail, field_name));
    }

    TeamCard {
        name: clean_text(&team.name),
        org: if team.organization.trim().is_empty() { String::new() } else { clean_text(&team.organization) },
        rows,
    }
}

/// Builds the renderable card element: a name / organisation header, a divider,
/// then one compact row per activity.
fn build_team_card(card: &TeamCard) -> elements::LinearLayout {
    let mut c = elements::LinearLayout::vertical();
    c.push(elements::Paragraph::new(card.name.clone()).styled(team_name_style()));
    if !card.org.is_empty() {
        c.push(elements::Paragraph::new(card.org.clone()).styled(team_org_style()));
    }
    c.push(elements::Break::new(0.2));
    c.push(rule_element(C_RULE, false));
    c.push(elements::Break::new(0.2));

    if card.rows.is_empty() {
        c.push(elements::Paragraph::new("No scheduled activities")
            .styled(style::Style::new().italic().with_font_size(FS_META).with_color(C_MUTED)));
        return c;
    }

    let mut table = elements::TableLayout::new(TEAM_WEIGHTS.to_vec());
    for (time, detail, field) in &card.rows {
        let mut row = table.row();
        row.push_element(elements::Paragraph::new(time.clone())
            .styled(style::Style::new().bold().with_font_size(FS_META)));
        row.push_element(elements::Paragraph::new(detail.clone())
            .styled(style::Style::new().with_font_size(FS_META)));
        row.push_element(elements::Paragraph::new(field.clone())
            .aligned(genpdf::Alignment::Right)
            .styled(style::Style::new().with_font_size(FS_META).with_color(C_MUTED)));
        row.push().ok();
    }
    c.push(table);
    c
}

/// Conservative rendered height of a card at the given content width, mirroring
/// the components built in [`build_team_card`] so the grid can keep a card whole.
fn card_height(context: &genpdf::Context, content_w: genpdf::Mm, card: &TeamCard, base_style: style::Style) -> genpdf::Mm {
    let name_lh = team_name_style().line_height(&context.font_cache);
    let meta_lh = team_org_style().line_height(&context.font_cache);
    let base_lh = base_style.line_height(&context.font_cache);

    let mut h = name_lh * wrapped_line_count(context, &team_name_style(), &card.name, content_w) as f64;
    if !card.org.is_empty() {
        h += meta_lh * wrapped_line_count(context, &team_org_style(), &card.org, content_w) as f64;
    }
    // The two 0.2-line breaks around the divider, plus the divider rule itself.
    h += base_lh * 0.4 + style::Style::new().with_font_size(1).line_height(&context.font_cache);

    if card.rows.is_empty() {
        h += meta_lh;
    } else {
        let weight_sum: usize = TEAM_WEIGHTS.iter().sum();
        let st = style::Style::new().with_font_size(FS_META);
        for (time, detail, field) in &card.rows {
            let lt = wrapped_line_count(context, &st, time, content_w * (TEAM_WEIGHTS[0] as f64 / weight_sum as f64));
            let ld = wrapped_line_count(context, &st, detail, content_w * (TEAM_WEIGHTS[1] as f64 / weight_sum as f64));
            let lf = wrapped_line_count(context, &st, field, content_w * (TEAM_WEIGHTS[2] as f64 / weight_sum as f64));
            h += meta_lh * lt.max(ld).max(lf) as f64;
        }
    }

    h + base_lh * 2.0 // safety margin so an estimate miss never splits a card
}

/// A two-up grid of team cards that never splits a card across a page boundary.
///
/// Like [`RepeatingScheduleTable`], it owns its content as data and paginates
/// during `render`: each pair of cards is measured against the remaining space
/// and deferred whole to the next page if it would not fit.
struct TeamGrid {
    title: String,
    cards: Vec<TeamCard>,
    next: usize,
    /// See [`RepeatingScheduleTable::deferred`].
    deferred: bool,
}

impl Element for TeamGrid {
    fn render(
        &mut self,
        context: &genpdf::Context,
        mut area: genpdf::render::Area<'_>,
        style: style::Style,
    ) -> Result<genpdf::RenderResult, genpdf::error::Error> {
        let mut result = genpdf::RenderResult::default();
        if self.next >= self.cards.len() {
            return Ok(result);
        }
        // Half the grid width, less the 4mm padding applied to each side of a card.
        let content_w = area.size().width * 0.5 - genpdf::Mm::from(8u8);

        // Keep the "Per-Team Schedule" title with the first row of cards.
        if self.next == 0 {
            let title_h = section_title_style().line_height(&context.font_cache)
                + style.line_height(&context.font_cache) * 0.6;
            let first_pair_h = card_height(context, content_w, &self.cards[0], style)
                .max(self.cards.get(1).map_or(genpdf::Mm::from(0u8), |r| card_height(context, content_w, r, style)));
            if !self.deferred && title_h + first_pair_h > area.size().height {
                self.deferred = true;
                result.has_more = true;
                return Ok(result);
            }
            let mut title = elements::Paragraph::new(self.title.clone()).styled(section_title_style());
            let title_result = title.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, title_result.size.height));
            result.size = result.size.stack_vertical(title_result.size);
            let mut brk = elements::Break::new(0.6);
            let brk_result = brk.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, brk_result.size.height));
            result.size = result.size.stack_vertical(brk_result.size);
        }

        let mut placed_any = false;
        while self.next < self.cards.len() {
            let left = &self.cards[self.next];
            let right = self.cards.get(self.next + 1);
            let pair_h = card_height(context, content_w, left, style)
                .max(right.map_or(genpdf::Mm::from(0u8), |r| card_height(context, content_w, r, style)));
            if pair_h > area.size().height && placed_any {
                break;
            }

            let mut row_table = elements::TableLayout::new(vec![1, 1]);
            let mut row = row_table.row();
            row.push_element(build_team_card(left).padded(4));
            match right {
                Some(r) => row.push_element(build_team_card(r).padded(4)),
                None => { row.push_element(elements::Paragraph::new("")); }
            }
            row.push().ok();

            let row_result = row_table.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, row_result.size.height));
            result.size = result.size.stack_vertical(row_result.size);
            placed_any = true;
            self.next += if right.is_some() { 2 } else { 1 };

            // Gap between card rows.
            let mut gap = elements::Break::new(0.6);
            let gap_result = gap.render(context, area.clone(), style)?;
            area.add_offset(genpdf::Position::new(0, gap_result.size.height));
            result.size = result.size.stack_vertical(gap_result.size);
        }

        result.has_more = self.next < self.cards.len();
        Ok(result)
    }
}

fn generate_pdf_document_internal(config: &crate::model::TournamentConfig, title: &str, assignments: &[ScheduleAssignment], report_type: ReportType, time_12h: bool) -> Option<genpdf::Document> {
    let font_family = load_embedded_font()?;

    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(format!("Schedule: {}", title));

    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(15);
    doc.set_page_decorator(decorator);

    // --- Masthead: competition + report title on the left, summary stats on
    //     the right, closed by a heavy primary rule. ---
    let mut header_table = elements::TableLayout::new(vec![2, 1]);
    let mut header_row = header_table.row();

    let mut left_header = elements::LinearLayout::vertical();
    left_header.push(elements::Paragraph::new(clean_text(&config.competition_name))
        .styled(style::Style::new().bold().with_font_size(FS_TITLE).with_color(C_PRIMARY)));
    left_header.push(elements::Paragraph::new(clean_text(&title.replace('_', " ")))
        .styled(style::Style::new().bold().with_font_size(FS_SUBTITLE).with_color(C_ACCENT)));
    header_row.push_element(left_header);

    let total_matches = assignments.len();
    let total_rounds = assignments.iter().map(|a| a.activity.round_index()).collect::<HashSet<_>>().len();
    let total_fields = assignments.iter().filter_map(|a| a.field_id.as_ref()).collect::<HashSet<_>>().len();

    let stat_style = style::Style::new().with_font_size(FS_META).with_color(C_MUTED);
    let mut right_header = elements::LinearLayout::vertical();
    for stat in [
        format!("{} rounds", total_rounds),
        format!("{} fields", total_fields),
        format!("{} activities", total_matches),
    ] {
        right_header.push(elements::Paragraph::new(stat).aligned(genpdf::Alignment::Right).styled(stat_style));
    }
    header_row.push_element(right_header);
    header_row.push().ok();

    doc.push(header_table);
    doc.push(elements::Break::new(0.4));
    doc.push(rule_element(C_PRIMARY, true));
    doc.push(elements::Break::new(1.0));

    // --- Schedule tables: the chronological list of activities. For master and
    //     division reports this is the full schedule; for a single team or
    //     volunteer it is just their own games, which is the whole report. ---
    let comp_assigns: Vec<_> = assignments.iter().filter(|a| !matches!(a.activity, crate::model::Activity::Interview { .. })).collect();
    let int_assigns: Vec<_> = assignments.iter().filter(|a| matches!(a.activity, crate::model::Activity::Interview { .. })).collect();

    render_schedule_section(&mut doc, config, "Competition Schedule", &comp_assigns, time_12h);
    render_schedule_section(&mut doc, config, "Interview Schedule", &int_assigns, time_12h);

    // --- Per-team grid: only for the broad master / division reports. ---
    if matches!(report_type, ReportType::Master | ReportType::Division) {
        let teams_to_show: Vec<&Team> = if matches!(report_type, ReportType::Division) {
            let div_id = assignments.first().map(|a| a.activity.division_id());
            config.teams.iter().filter(|t| Some(t.division_id.as_str()) == div_id).collect()
        } else {
            let team_names: HashSet<_> = assignments.iter().flat_map(|a| a.activity.teams()).collect();
            let mut teams: Vec<_> = config.teams.iter().filter(|t| team_names.contains(t.name.as_str())).collect();
            teams.sort_by_key(|t| &t.name);
            teams
        };

        let cards: Vec<TeamCard> = teams_to_show.iter()
            .map(|team| prepare_team_card(config, team, assignments, time_12h))
            .collect();
        if !cards.is_empty() {
            doc.push(elements::Break::new(1.0));
            doc.push(TeamGrid { title: "Per-Team Schedule".to_string(), cards, next: 0, deferred: false });
        }
    }

    // --- Footer ---
    doc.push(elements::Break::new(1.0));
    doc.push(rule_element(C_RULE, false));
    doc.push(elements::Break::new(0.3));
    doc.push(elements::Paragraph::new(format!("Generated {}", chrono::Local::now().format("%Y-%m-%d %H:%M")))
        .aligned(genpdf::Alignment::Right)
        .styled(style::Style::new().with_font_size(FS_FOOTER).with_color(C_FOOTER)));

    Some(doc)
}
impl AppState {
    pub fn prepare_csv_import(&mut self, content: &str) {
        let mut lines = content.lines();
        let header = match lines.next() {
            Some(h) => h,
            None => {
                self.status_message = "Empty CSV file".to_string();
                return;
            }
        };

        if !header.contains("Name") || !header.contains("Division") {
            self.status_message = "Invalid CSV: Missing 'Name' or 'Division' columns".to_string();
            return;
        }

        let mut divisions_found = HashSet::new();
        for line in lines {
            let parts = split_csv_line(line);
            // Match finalize_csv_import's requirement so divisions only appear in
            // the picker for rows that will actually be importable.
            if parts.len() < 11 { continue; }
            let div_name = parts[6].trim();
            if !div_name.is_empty() {
                divisions_found.insert(div_name.to_string());
            }
        }

        let mut sorted_divs: Vec<String> = divisions_found.into_iter().collect();
        sorted_divs.sort();

        self.csv_import = Some(super::app_state::CsvImportData {
            raw_content: content.to_string(),
            divisions: sorted_divs.clone(),
            selected_divisions: sorted_divs.into_iter().collect(), // Select all by default
        });
    }

    pub fn finalize_csv_import(&mut self) {
        let import_data = match self.csv_import.take() {
            Some(data) => data,
            None => return,
        };

        let mut lines = import_data.raw_content.lines();
        lines.next(); // Skip header

        let mut count = 0;
        for line in lines {
            if line.trim().is_empty() { continue; }
            let parts = split_csv_line(line);
            if parts.len() < 11 { continue; }

            let name = parts[3].trim();
            let withdrawn = parts[4].trim().to_lowercase() == "true";
            let division_name = parts[6].trim();
            let school = parts[10].trim();

            if withdrawn || name.is_empty() { continue; }
            if !import_data.selected_divisions.contains(division_name) { continue; }

            let div_id = self.get_or_create_division_from_name(division_name);

            self.config.teams.push(Team {
                name: name.to_string(),
                division_id: div_id,
                organization: school.to_string(),
            });
            count += 1;
        }

        self.status_message = format!("Successfully imported {} teams!", count);
        self.clear_schedule();
        self.update_diagnostics();
    }

    fn get_or_create_division_from_name(&mut self, name: &str) -> String {
        use crate::scheduler::sanitize_name;
        use crate::model::{Division, SchedulingMode};

        // Check if exists
        for div in &self.config.divisions {
            if div.name == name {
                return div.id.clone();
            }
        }

        // Create new
        let existing_ids: Vec<String> = self.config.divisions.iter().map(|d| d.id.clone()).collect();
        let id = crate::scheduler::unique_id(&sanitize_name(name), &existing_ids);
        let mode = if name.to_lowercase().contains("soccer") {
            SchedulingMode::HeadToHead
        } else {
            SchedulingMode::IndividualRun
        };

        use rand::Rng;
        let mut rng = rand::thread_rng();
        let random_color = [
            rng.gen_range(50..=200),
            rng.gen_range(50..=200),
            rng.gen_range(50..=200),
        ];

        self.config.divisions.push(Division {
            id: id.clone(),
            name: name.to_string(),
            mode,
            games_per_team: 3,
            volunteers_required: if mode == SchedulingMode::HeadToHead { 2 } else { 1 },
            duration_minutes: 20,
            allowed_fields: None,
            interviews_enabled: true,
            interview_volunteers_required: if mode == SchedulingMode::HeadToHead { 2 } else { 1 },
            interview_duration_minutes: 15,
            finals_enabled: false,
            finals_rounds: None,
            finals_duration_minutes: None,
            finals_third_place_playoff: false,
            color: Some(random_color), min_match_break_minutes: None,
        });
        id
    }
}

fn clean_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .filter(|c| c.is_ascii()) // Remove emojis/non-ascii
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn clean_text(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii()).collect()
}

/// Escapes a value for a quoted CSV field per RFC 4180: doubles any embedded
/// quotes. The caller is responsible for wrapping the result in `"`. Without
/// this, a name containing `"` or a newline corrupts the row layout.
fn csv_field(value: &str) -> String {
    value.replace('"', "\"\"")
}

/// First three characters of a day name (e.g. "Saturday" -> "Sat"), falling back
/// to the whole string when shorter. Uses char boundaries so non-ASCII or short
/// day strings can never panic the way `&day[..3]` would.
fn day_abbr(day: &str) -> &str {
    day.get(..3).unwrap_or(day)
}

/// Formats a stored `"HH:MM"` time for export. With `twelve_hour` it becomes a
/// 12-hour clock with an AM/PM suffix (e.g. `"14:30"` -> `"2:30 PM"`), which is
/// clearer for young participants; otherwise the original 24-hour string is
/// returned. Unparseable input is passed through unchanged.
fn format_time_str(time: &str, twelve_hour: bool) -> String {
    if !twelve_hour {
        return time.to_string();
    }
    let parts: Vec<&str> = time.split(':').collect();
    let (h, m) = match parts.as_slice() {
        [h, m] => match (h.parse::<u32>(), m.parse::<u32>()) {
            (Ok(h), Ok(m)) if h < 24 && m < 60 => (h, m),
            _ => return time.to_string(),
        },
        _ => return time.to_string(),
    };
    let suffix = if h < 12 { "AM" } else { "PM" };
    let h12 = match h % 12 { 0 => 12, x => x };
    format!("{}:{:02} {}", h12, m, suffix)
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars = line.chars().peekable();

    for c in chars {
        match c {
            '\"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                result.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    result.push(current);
    result
}

#[cfg(test)]
mod time_fmt_test {
    use super::*;
    #[test]
    fn twelve_hour_formatting() {
        assert_eq!(format_time_str("09:00", false), "09:00");
        assert_eq!(format_time_str("09:00", true), "9:00 AM");
        assert_eq!(format_time_str("14:20", true), "2:20 PM");
        assert_eq!(format_time_str("00:00", true), "12:00 AM");
        assert_eq!(format_time_str("12:00", true), "12:00 PM");
        assert_eq!(format_time_str("23:59", true), "11:59 PM");
        assert_eq!(format_time_str("bogus", true), "bogus");
        assert_eq!(format_time_str("25:00", true), "25:00");
    }
}
