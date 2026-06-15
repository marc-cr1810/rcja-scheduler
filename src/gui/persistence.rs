use std::fs::{self, File};
use std::io::Write;
use std::collections::HashSet;
use crate::model::{Team, ScheduleAssignment};
use super::{AppState, ExportMessage};
use genpdf::{elements, style, Element};

#[derive(Clone, Copy)]
enum ReportType {
    Master,
    Division,
    Team,
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
            let csv = generate_csv_content_internal(&self.config, &schedule.assignments);
            
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
                        csv_field(&vol.name), csv_field(&slot.start_time), csv_field(&slot.day),
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
            let tournament_name = clean_filename(&config.competition_name);
            let root = base_path.join(format!("Export_{}", tournament_name));

            std::thread::spawn(move || {
                if let Err(e) = fs::create_dir_all(&root) {
                    let _ = tx.send(ExportMessage::Error(format!("Failed to create directory: {}", e)));
                    return;
                }

                // Top-level format folders
                let csv_root = root.join("csv");
                let pdf_root = root.join("pdf");
                let _ = fs::create_dir_all(&csv_root);
                let _ = fs::create_dir_all(&pdf_root);

                // Subdirectories within format folders
                let csv_div = csv_root.join("Divisions");
                let csv_team = csv_root.join("Teams");
                let pdf_div = pdf_root.join("Divisions");
                let pdf_team = pdf_root.join("Teams");
                
                let _ = fs::create_dir_all(&csv_div);
                let _ = fs::create_dir_all(&csv_team);
                let _ = fs::create_dir_all(&pdf_div);
                let _ = fs::create_dir_all(&pdf_team);

                let divisions = config.divisions.clone();
                let teams = config.teams.clone();
                let total_steps = 1 + divisions.len() + teams.len();
                let mut current_step = 0;

                let writer = ExportWriter {
                    config: config.clone(),
                };

                // 1. Master Schedules
                writer.write_export_files(&csv_root, &pdf_root, "Master_Schedule", &schedule.assignments, ReportType::Master);
                current_step += 1;
                let _ = tx.send(ExportMessage::Progress(current_step as f32 / total_steps as f32));

                // 2. Division Schedules
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

                // 3. Team Schedules
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

                let _ = tx.send(ExportMessage::Done(format!("Full export completed to '{}'", root.display())));
            });
        }
    }
}

struct ExportWriter {
    config: crate::model::TournamentConfig,
}

impl ExportWriter {
    fn write_export_files(&self, csv_dir: &std::path::Path, pdf_dir: &std::path::Path, name: &str, assignments: &[ScheduleAssignment], report_type: ReportType) {
        // CSV
        let csv_content = generate_csv_content_internal(&self.config, assignments);
        let csv_path = csv_dir.join(format!("{}.csv", name));
        let _ = fs::write(csv_path, csv_content);

        // PDF
        let pdf_path = pdf_dir.join(format!("{}.pdf", name));
        
        match generate_pdf_document_internal(&self.config, name, assignments, report_type) {
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

fn generate_csv_content_internal(config: &crate::model::TournamentConfig, assignments: &[ScheduleAssignment]) -> String {
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
        
        let time_str = slot.map_or("".to_string(), |s| format!("{} {}", day_abbr(&s.day), s.start_time));
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

/// Cell decorator for the main schedule tables: a heavy rule under the header
/// row (row 0) and a light rule under every body row, giving clean ledger-style
/// separation without any filled backgrounds.
struct ScheduleRuleDecorator;

impl genpdf::elements::CellDecorator for ScheduleRuleDecorator {
    fn decorate_cell(&mut self, _column: usize, row: usize, _has_more: bool, area: genpdf::render::Area<'_>, _style: style::Style) {
        let h = area.size().height;
        if row == 0 {
            draw_hline(&area, h, C_PRIMARY);
            draw_hline(&area, h * 0.985, C_PRIMARY);
        } else {
            draw_hline(&area, h, C_RULE);
        }
    }
}

fn render_schedule_section(doc: &mut genpdf::Document, config: &crate::model::TournamentConfig, section_title: &str, assignments: &[&ScheduleAssignment]) {
    if assignments.is_empty() { return; }

    doc.push(elements::Paragraph::new(section_title)
        .styled(style::Style::new().bold().with_font_size(FS_SECTION).with_color(C_PRIMARY)));
    doc.push(elements::Break::new(0.4));

    // One row per assignment, sorted by day / time / field, with a colour-coded
    // header underline and consistent column widths.
    let mut table = elements::TableLayout::new(vec![4, 3, 3, 8, 6]);
    table.set_cell_decorator(ScheduleRuleDecorator);

    let th = style::Style::new().bold().with_font_size(FS_TH).with_color(C_PRIMARY);
    let mut h_row = table.row();
    for label in ["TIME", "ROUND", "FIELD", "ACTIVITY", "DIVISION"] {
        h_row.push_element(elements::Paragraph::new(label).styled(th).padded(2));
    }
    h_row.push().ok();

    let mut sorted: Vec<&ScheduleAssignment> = assignments.to_vec();
    sorted.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        (
            slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time))),
            a.field_id.clone(),
        )
    });

    for assign in sorted {
        let slot = match config.time_slots.iter().find(|s| s.id == assign.time_slot_id) {
            Some(s) => s,
            None => continue,
        };
        let field_name = assign.field_id.as_ref()
            .and_then(|fid| config.fields.iter().find(|f| f.id == *fid))
            .map_or_else(|| "-".to_string(), |f| clean_text(&f.name));

        let time_str = format!("{} {}", day_abbr(&slot.day), slot.start_time);

        let mut row = table.row();
        row.push_element(elements::Paragraph::new(time_str)
            .styled(style::Style::new().bold().with_font_size(FS_BODY)).padded(2));
        row.push_element(elements::Paragraph::new(assign.activity.round_label())
            .styled(style::Style::new().with_font_size(FS_BODY).with_color(C_MUTED)).padded(2));
        row.push_element(elements::Paragraph::new(field_name)
            .styled(style::Style::new().with_font_size(FS_BODY)).padded(2));
        row.push_element(elements::Paragraph::new(clean_text(&assign.activity.export_label()))
            .styled(style::Style::new().bold().with_font_size(FS_BODY)).padded(2));
        row.push_element(elements::Paragraph::new(clean_text(&get_activity_metadata(config, &assign.activity)))
            .styled(style::Style::new().with_font_size(FS_META).with_color(C_MUTED)).padded(2));
        row.push().ok();
    }
    doc.push(table);
    doc.push(elements::Break::new(1.0));
}

/// Renders a single team's personal schedule as a self-contained card: a name /
/// organisation header, a divider, then one compact row per activity.
fn render_team_card(config: &crate::model::TournamentConfig, team: &Team, assignments: &[ScheduleAssignment]) -> elements::LinearLayout {
    let mut card = elements::LinearLayout::vertical();
    card.push(elements::Paragraph::new(clean_text(&team.name))
        .styled(style::Style::new().bold().with_font_size(FS_BODY + 1).with_color(C_PRIMARY)));
    if !team.organization.trim().is_empty() {
        card.push(elements::Paragraph::new(clean_text(&team.organization))
            .styled(style::Style::new().with_font_size(FS_META).with_color(C_MUTED)));
    }
    card.push(elements::Break::new(0.2));
    card.push(rule_element(C_RULE, false));
    card.push(elements::Break::new(0.2));

    let mut team_assigns: Vec<_> = assignments.iter()
        .filter(|a| a.activity.teams().contains(&team.name.as_str()))
        .collect();
    team_assigns.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
    });

    if team_assigns.is_empty() {
        card.push(elements::Paragraph::new("No scheduled activities")
            .styled(style::Style::new().italic().with_font_size(FS_META).with_color(C_MUTED)));
        return card;
    }

    let mut table = elements::TableLayout::new(vec![4, 8, 5]);
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

        let time_str = format!("{} {}", day_abbr(&slot.day), slot.start_time);
        let mut row = table.row();
        row.push_element(elements::Paragraph::new(time_str)
            .styled(style::Style::new().bold().with_font_size(FS_META)));
        row.push_element(elements::Paragraph::new(detail)
            .styled(style::Style::new().with_font_size(FS_META)));
        row.push_element(elements::Paragraph::new(field_name)
            .aligned(genpdf::Alignment::Right)
            .styled(style::Style::new().with_font_size(FS_META).with_color(C_MUTED)));
        row.push().ok();
    }
    card.push(table);
    card
}

fn generate_pdf_document_internal(config: &crate::model::TournamentConfig, title: &str, assignments: &[ScheduleAssignment], report_type: ReportType) -> Option<genpdf::Document> {
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

    // --- Schedule tables (master / division reports) ---
    if !matches!(report_type, ReportType::Team) {
        let comp_assigns: Vec<_> = assignments.iter().filter(|a| !matches!(a.activity, crate::model::Activity::Interview { .. })).collect();
        let int_assigns: Vec<_> = assignments.iter().filter(|a| matches!(a.activity, crate::model::Activity::Interview { .. })).collect();

        render_schedule_section(&mut doc, config, "Competition Schedule", &comp_assigns);
        render_schedule_section(&mut doc, config, "Interview Schedule", &int_assigns);
    }

    // --- Per-team schedules: a two-up grid of self-contained cards. ---
    if !matches!(report_type, ReportType::Team) {
        doc.push(elements::Break::new(1.0));
        doc.push(elements::Paragraph::new("Per-Team Schedule")
            .styled(style::Style::new().bold().with_font_size(FS_SECTION).with_color(C_PRIMARY)));
        doc.push(elements::Break::new(0.6));
    }

    let teams_to_show: Vec<&Team> = if matches!(report_type, ReportType::Team) {
        let team_names: HashSet<_> = assignments.iter().flat_map(|a| a.activity.teams()).collect();
        config.teams.iter().filter(|t| team_names.contains(t.name.as_str())).collect()
    } else if matches!(report_type, ReportType::Division) {
        let div_id = assignments.first().map(|a| a.activity.division_id());
        config.teams.iter().filter(|t| Some(t.division_id.as_str()) == div_id).collect()
    } else {
        let team_names: HashSet<_> = assignments.iter().flat_map(|a| a.activity.teams()).collect();
        let mut teams: Vec<_> = config.teams.iter().filter(|t| team_names.contains(t.name.as_str())).collect();
        teams.sort_by_key(|t| &t.name);
        teams
    };

    let mut teams_iter = teams_to_show.into_iter().peekable();
    while let Some(team1) = teams_iter.next() {
        let team2 = teams_iter.next();

        let mut row_table = elements::TableLayout::new(vec![1, 1]);
        let mut row = row_table.row();
        for team in [Some(team1), team2] {
            match team {
                Some(team) => row.push_element(render_team_card(config, team, assignments).padded(4)),
                None => { row.push_element(elements::Paragraph::new("")); }
            }
        }
        row.push().ok();
        doc.push(row_table);
        doc.push(elements::Break::new(0.6));
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
