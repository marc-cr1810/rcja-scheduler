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
    let mut csv = String::from("Day,Start Time,End Time,Division,Activity,Teams,Field,Volunteers\n");
    
    let mut sorted_assigns = assignments.to_vec();
    sorted_assigns.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        slot.map(|s| (s.day.clone(), s.start_minutes()))
    });

    for assign in sorted_assigns {
        let slot = config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
        let field = assign.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
        let division = config.divisions.iter().find(|d| d.id == assign.activity.division_id());
        
        let day = slot.map_or("", |s| &s.day);
        let start = slot.map_or("", |s| &s.start_time);
        let end = slot.map_or("", |s| &s.end_time);
        let div_name = division.map_or("", |d| &d.name);
        let act_label = assign.activity.export_label();
        let teams = assign.activity.teams().join(" vs ");
        let field_name = field.map_or("", |f| &f.name);
        
        let vols: Vec<String> = assign.volunteer_ids.iter()
            .map(|vid| config.volunteers.iter().find(|v| v.id == *vid).map_or(vid.clone(), |v| v.name.clone()))
            .collect();
        let vol_names = vols.join("; ");

        csv.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            day, start, end, div_name, act_label, teams, field_name, vol_names
        ));
    }
    csv
}

fn generate_pdf_document_internal(config: &crate::model::TournamentConfig, title: &str, assignments: &[ScheduleAssignment], report_type: ReportType) -> Option<genpdf::Document> {
    // Load system font
    let font_dir = "/usr/share/fonts/truetype/ubuntu";
    let font_family = genpdf::fonts::from_files(font_dir, "Ubuntu", None).ok()
        .or_else(|| genpdf::fonts::from_files("/usr/share/fonts/truetype/liberation", "LiberationSans", None).ok())
        .or_else(|| genpdf::fonts::from_files("/usr/share/fonts/truetype/dejavu", "DejaVuSans", None).ok());

    let font_family = match font_family {
        Some(f) => f,
        None => return None,
    };

    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(format!("Schedule: {}", title));
    
    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(15);
    doc.set_page_decorator(decorator);

    // Header Section
    let comp_name = clean_text(&config.competition_name);
    let display_title = clean_text(&title.replace('_', " "));

    doc.push(elements::Paragraph::new(comp_name)
        .styled(style::Style::new().bold().with_font_size(22).with_color(style::Color::Rgb(30, 58, 138))));
    
    doc.push(elements::Paragraph::new(display_title)
        .styled(style::Style::new().bold().with_font_size(16).with_color(style::Color::Rgb(79, 70, 229))));
    
    doc.push(elements::Break::new(0.5));
    doc.push(elements::Paragraph::new("________________________________________________________________________________").styled(style::Color::Rgb(226, 232, 240)));
    doc.push(elements::Break::new(1.0));

    // Define columns based on report type
    let (column_weights, headers) = match report_type {
        ReportType::Master => (
            vec![3, 5, 10, 5, 6],
            vec!["Time", "Division", "Activity / Teams", "Location", "Volunteers"]
        ),
        ReportType::Division => (
            vec![3, 10, 5, 6],
            vec!["Time", "Activity / Teams", "Location", "Volunteers"]
        ),
        ReportType::Team => (
            vec![3, 5, 10, 5, 6],
            vec!["Time", "Division", "Match Details", "Location", "Volunteers"]
        ),
    };

    let mut table = elements::TableLayout::new(column_weights);
    table.set_cell_decorator(elements::FrameCellDecorator::new(true, true, false));
    
    // Header Row
    let mut header_row = table.row();
    let header_style = style::Style::new().bold().with_font_size(11);
    for h in headers {
        header_row.push_element(elements::Paragraph::new(h).styled(header_style).padded(2));
    }
    header_row.push().expect("Failed to add header row");

    let mut sorted_assigns = assignments.to_vec();
    sorted_assigns.sort_by_key(|a| {
        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
        slot.map(|s| (s.day.clone(), crate::gui::helpers::parse_time_to_minutes(&s.start_time)))
    });

    for assign in sorted_assigns {
        let slot = config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
        let field = assign.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
        let division = config.divisions.iter().find(|d| d.id == assign.activity.division_id());
        
        let time_str = slot.map_or("?".to_string(), |s| format!("{} {}", &s.day[..3], s.start_time));
        let div_name = clean_text(division.map_or("—", |d| &d.name));
        let act_label = clean_text(&assign.activity.export_label());
        let field_name = clean_text(field.map_or("—", |f| &f.name));
        
        let vols: Vec<String> = assign.volunteer_ids.iter()
            .map(|vid| config.volunteers.iter().find(|v| v.id == *vid).map_or(vid.clone(), |v| clean_text(&v.name)))
            .collect();
        let vol_names = vols.join(", ");

        let mut row = table.row();
        let cell_style = style::Style::new().with_font_size(10);
        
        row.push_element(elements::Paragraph::new(time_str).styled(cell_style).padded(2));
        
        match report_type {
            ReportType::Master => {
                row.push_element(elements::Paragraph::new(div_name).styled(cell_style).padded(2));
                row.push_element(elements::Paragraph::new(act_label).styled(cell_style).padded(2));
            }
            ReportType::Division => {
                row.push_element(elements::Paragraph::new(act_label).styled(cell_style).padded(2));
            }
            ReportType::Team => {
                row.push_element(elements::Paragraph::new(div_name).styled(cell_style).padded(2));
                row.push_element(elements::Paragraph::new(act_label).styled(cell_style).padded(2));
            }
        }
        
        row.push_element(elements::Paragraph::new(field_name).styled(cell_style).padded(2));
        row.push_element(elements::Paragraph::new(vol_names).styled(style::Style::new().with_font_size(9)).padded(2));
        row.push().expect("Failed to add row");
    }

    doc.push(table);
    
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
