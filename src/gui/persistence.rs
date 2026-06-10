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
        if let Ok(json) = serde_json::to_string_pretty(&self.config) {
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
                    let slot = self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id).unwrap();
                    let field = assign.field_id.as_ref().and_then(|fid| self.config.fields.iter().find(|f| f.id == *fid));
                    let div = self.config.divisions.iter().find(|d| d.id == assign.activity.division_id());
                    
                    csv.push_str(&format!(
                        "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
                        vol.name, slot.start_time, slot.day, field.map_or("", |f| &f.name), assign.activity.export_label(), div.map_or("", |d| &d.name)
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
        
        let time_str = slot.map_or("".to_string(), |s| format!("{} {}", &s.day[..3], s.start_time));
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
            field_name, time_str, div_name, assign.activity.export_label(), team_a, org_a, team_b, org_b, vol_names
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

struct ScheduleDecorator {
    color_header: style::Color,
    start_index: usize,
}

impl genpdf::elements::CellDecorator for ScheduleDecorator {
    fn decorate_cell(&mut self, _column: usize, row: usize, _is_last_row: bool, area: genpdf::render::Area<'_>, _style: style::Style) {
        let size = area.size();
        let actual_row = row + self.start_index;

        if actual_row == 0 {
            // Fill header background by drawing lines (hack for genpdf 0.2.0)
            let steps = 40; // Reduced density to improve rendering
            for i in 0..steps {
                let y = size.height * (i as f64 / steps as f64);
                area.draw_line(
                    vec![
                        genpdf::Position { x: 0.into(), y },
                        genpdf::Position { x: size.width, y },
                    ],
                    style::Style::new().with_color(self.color_header),
                );
            }
        }

        // Draw horizontal line at the bottom
        let line_color = if actual_row == 0 {
            self.color_header
        } else {
            style::Color::Rgb(226, 232, 240)
        };

        area.draw_line(
            vec![
                genpdf::Position { x: 0.into(), y: size.height },
                genpdf::Position { x: size.width, y: size.height },
            ],
            style::Style::new().with_color(line_color),
        );
    }
}

struct TeamCardDecorator {
    color_border: style::Color,
}

impl genpdf::elements::CellDecorator for TeamCardDecorator {
    fn decorate_cell(&mut self, _column: usize, _row: usize, _is_last_row: bool, area: genpdf::render::Area<'_>, _style: style::Style) {
        let size = area.size();
        // Just a subtle bottom border for the card
        area.draw_line(
            vec![
                genpdf::Position { x: 0.into(), y: size.height },
                genpdf::Position { x: size.width, y: size.height },
            ],
            style::Style::new().with_color(self.color_border),
        );
    }
}

fn render_schedule_section(doc: &mut genpdf::Document, config: &crate::model::TournamentConfig, section_title: &str, assignments: &[&ScheduleAssignment], color_header: style::Color, color_gray: style::Color) {
    if assignments.is_empty() { return; }

    doc.push(elements::Paragraph::new(section_title)
        .styled(style::Style::new().bold().with_font_size(16).with_color(color_header)));
    doc.push(elements::Break::new(0.5));

    let mut table = elements::TableLayout::new(vec![3, 3, 14]);
    table.set_cell_decorator(ScheduleDecorator { color_header, start_index: 0 });
    
    // Header Row
    let mut h_row = table.row();
    let h_style = style::Style::new().bold().with_font_size(10).with_color(style::Color::Rgb(255, 255, 255));
    h_row.push_element(elements::Paragraph::new("TIME").styled(h_style).padded(2));
    h_row.push_element(elements::Paragraph::new("ROUND").styled(h_style).padded(2));
    h_row.push_element(elements::Paragraph::new("ACTIVITIES / MATCHES").styled(h_style).padded(2));
    h_row.push().ok();

    // Group by Time Slot
    let mut slot_ids: Vec<_> = assignments.iter().map(|a| &a.time_slot_id).collect::<HashSet<_>>().into_iter().collect();
    slot_ids.sort_by_key(|id| {
        let slot = config.time_slots.iter().find(|s| s.id == **id);
        slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
    });

    for slot_id in slot_ids {
        let slot = config.time_slots.iter().find(|s| s.id == *slot_id).unwrap();
        let mut slot_assigns: Vec<_> = assignments.iter().filter(|a| a.time_slot_id == *slot_id).collect();
        slot_assigns.sort_by_key(|a| a.field_id.clone());

        let time_str = format!("{} {}", &slot.day[..3], slot.start_time);
        let round_str = slot_assigns.first().map_or("".to_string(), |a| a.activity.round_label());

        let mut row = table.row();
        row.push_element(elements::Paragraph::new(time_str).styled(style::Style::new().bold().with_font_size(10)).padded(2));
        row.push_element(elements::Paragraph::new(round_str).styled(style::Style::new().with_font_size(10)).padded(2));

        let mut details = elements::LinearLayout::vertical();
        for (i, assign) in slot_assigns.into_iter().enumerate() {
            if i > 0 { details.push(elements::Break::new(0.2)); }

            let field_name = assign.field_id.as_ref()
                .and_then(|fid| config.fields.iter().find(|f| f.id == *fid))
                .map_or("—", |f| &f.name);

            let match_text = format!("● {} ({})", clean_text(&assign.activity.export_label()), clean_text(field_name));
            details.push(elements::Paragraph::new(match_text)
                .styled(style::Style::new().bold().with_font_size(10)));

            let metadata = get_activity_metadata(config, &assign.activity);
            details.push(elements::Paragraph::new(format!("  {}", clean_text(&metadata)))
                .styled(style::Style::new().with_font_size(8).with_color(color_gray)));
        }
        row.push_element(details.padded(2));
        row.push().ok();
    }
    doc.push(table);
    doc.push(elements::Break::new(1.0));
}

fn generate_pdf_document_internal(config: &crate::model::TournamentConfig, title: &str, assignments: &[ScheduleAssignment], report_type: ReportType) -> Option<genpdf::Document> {
    // Colors
    let color_header = style::Color::Rgb(30, 58, 138); // Deep blue
    let color_accent = style::Color::Rgb(79, 70, 229); // Accent blue/purple
    let color_gray = style::Color::Rgb(107, 114, 128); // Gray for secondary info
    let color_border = style::Color::Rgb(226, 232, 240); // Light border color

    // Load system font (Cross-platform)
    let font_family = if cfg!(windows) {
        let win_font_dir = "C:\\Windows\\Fonts";
        genpdf::fonts::from_files(win_font_dir, "Arial", None).ok()
            .or_else(|| genpdf::fonts::from_files(win_font_dir, "Segoe UI", None).ok())
            .or_else(|| genpdf::fonts::from_files(win_font_dir, "Times New Roman", None).ok())
    } else {
        let font_dir = "/usr/share/fonts/truetype/ubuntu";
        genpdf::fonts::from_files(font_dir, "Ubuntu", None).ok()
            .or_else(|| genpdf::fonts::from_files("/usr/share/fonts/truetype/liberation", "LiberationSans", None).ok())
            .or_else(|| genpdf::fonts::from_files("/usr/share/fonts/truetype/dejavu", "DejaVuSans", None).ok())
            .or_else(|| genpdf::fonts::from_files("/usr/share/fonts/truetype/freefont", "FreeSans", None).ok())
    };

    let font_family = match font_family {
        Some(f) => f,
        None => return None,
    };

    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(format!("Schedule: {}", title));

    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(15);
    doc.set_page_decorator(decorator);

    // --- Header Section ---
    let mut header_table = elements::TableLayout::new(vec![1, 1]);
    let mut header_row = header_table.row();

    let mut left_header = elements::LinearLayout::vertical();
    left_header.push(elements::Paragraph::new(clean_text(&config.competition_name))
        .styled(style::Style::new().bold().with_font_size(22).with_color(color_header)));
    left_header.push(elements::Paragraph::new(clean_text(&title.replace('_', " ")))
        .styled(style::Style::new().bold().with_font_size(16).with_color(color_accent)));
    header_row.push_element(left_header);

    let mut right_header = elements::LinearLayout::vertical();
    let total_matches = assignments.len();
    let rounds: HashSet<_> = assignments.iter().map(|a| a.activity.round_index()).collect();
    let total_rounds = rounds.len();
    let fields: HashSet<_> = assignments.iter().filter_map(|a| a.field_id.as_ref()).collect();
    let total_fields = fields.len();

    right_header.push(elements::Paragraph::new(format!("{} rounds  {} fields  {} matches", total_rounds, total_fields, total_matches))
        .aligned(genpdf::Alignment::Right)
        .styled(style::Style::new().with_font_size(10).with_color(color_gray)));
    header_row.push_element(right_header);
    header_row.push().ok();

    doc.push(header_table);
    doc.push(elements::Break::new(0.5));
    doc.push(elements::Paragraph::new("________________________________________________________________________________")
        .styled(style::Color::Rgb(226, 232, 240)));
    doc.push(elements::Break::new(1.0));

    // --- Main Schedule Table ---
    if !matches!(report_type, ReportType::Team) {
        let comp_assigns: Vec<_> = assignments.iter().filter(|a| !matches!(a.activity, crate::model::Activity::Interview { .. })).collect();
        let int_assigns: Vec<_> = assignments.iter().filter(|a| matches!(a.activity, crate::model::Activity::Interview { .. })).collect();

        render_schedule_section(&mut doc, config, "Competition Schedule", &comp_assigns, color_header, color_gray);
        render_schedule_section(&mut doc, config, "Interview Schedule", &int_assigns, color_header, color_gray);
    }

    // --- Per-Team Schedule Section ---
    if matches!(report_type, ReportType::Master) || matches!(report_type, ReportType::Division) || matches!(report_type, ReportType::Team) {
        if !matches!(report_type, ReportType::Team) {
            doc.push(elements::Break::new(2.0));
            doc.push(elements::Paragraph::new("Per-Team Schedule")
                .styled(style::Style::new().bold().with_font_size(16).with_color(color_header)));
            doc.push(elements::Break::new(1.0));
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
            row_table.set_cell_decorator(TeamCardDecorator { color_border });
            let mut row = row_table.row();

            for team in [Some(team1), team2] {
                if let Some(team) = team {
                    let mut card = elements::LinearLayout::vertical();
                    card.push(elements::Paragraph::new(clean_text(&team.name))
                        .styled(style::Style::new().bold().with_font_size(11).with_color(color_header)));
                    card.push(elements::Paragraph::new(clean_text(&team.organization))
                        .styled(style::Style::new().with_font_size(9).with_color(color_gray)));
                    card.push(elements::Break::new(0.5));

                    let mut team_assigns: Vec<_> = assignments.iter()
                        .filter(|a| a.activity.teams().contains(&team.name.as_str()))
                        .collect();

                    team_assigns.sort_by_key(|a| {
                        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                        slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
                    });

                    for assign in team_assigns {
                        let slot = config.time_slots.iter().find(|s| s.id == assign.time_slot_id).unwrap();
                        let field_name = assign.field_id.as_ref()
                            .and_then(|fid| config.fields.iter().find(|f| f.id == *fid))
                            .map_or("—", |f| &f.name);

                        let mut match_line = elements::TableLayout::new(vec![4, 10, 5]);
                        let mut m_row = match_line.row();
                        let time_str = format!("{} {}", &slot.day[..3], slot.start_time);
                        m_row.push_element(elements::Paragraph::new(time_str).styled(style::Style::new().bold().with_font_size(9)));

                        let opponent = match &assign.activity {
                            crate::model::Activity::Match { team_a, team_b, .. } => {
                                let opp = if team_a == &team.name { team_b } else { team_a };
                                format!("vs {}", clean_text(opp))
                            }
                            crate::model::Activity::Run { run_number, .. } => format!("Run #{}", run_number),
                            crate::model::Activity::Interview { .. } => "Interview".to_string(),
                        };
                        m_row.push_element(elements::Paragraph::new(opponent).styled(style::Style::new().with_font_size(9)));
                        m_row.push_element(elements::Paragraph::new(clean_text(field_name))
                            .aligned(genpdf::Alignment::Right)
                            .styled(style::Style::new().with_font_size(8).with_color(color_gray)));
                        m_row.push().ok();
                        card.push(match_line);
                    }

                    row.push_element(card.padded(5));
                } else {
                    row.push_element(elements::Paragraph::new(""));
                }
            }
            row.push().ok();
            doc.push(row_table);
            doc.push(elements::Break::new(0.5));
        }
    }

    doc.push(elements::Break::new(1.5));
    doc.push(elements::Paragraph::new(format!("Generated on {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")))
        .styled(style::Style::new().with_font_size(8).with_color(style::Color::Rgb(148, 163, 184))));

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
            if parts.len() < 7 { continue; }
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
        let id = sanitize_name(name);
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
            color: Some(random_color),
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
