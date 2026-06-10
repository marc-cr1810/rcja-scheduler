use std::fs::File;
use std::io::Write;
use std::collections::HashSet;
use crate::model::Team;
use super::AppState;

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
            let mut csv = String::from("Day,Start Time,End Time,Division,Activity,Teams,Field,Volunteers\n");
            
            let mut assignments = schedule.assignments.clone();
            assignments.sort_by_key(|a| {
                let slot = self.config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                slot.map(|s| (s.day.clone(), s.start_minutes()))
            });

            for assign in assignments {
                let slot = self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
                let field = assign.field_id.as_ref().and_then(|fid| self.config.fields.iter().find(|f| f.id == *fid));
                let division = self.config.divisions.iter().find(|d| d.id == assign.activity.division_id());
                
                let day = slot.map_or("", |s| &s.day);
                let start = slot.map_or("", |s| &s.start_time);
                let end = slot.map_or("", |s| &s.end_time);
                let div_name = division.map_or("", |d| &d.name);
                let act_label = assign.activity.label();
                let teams = assign.activity.teams().join(" vs ");
                let field_name = field.map_or("", |f| &f.name);
                
                let vols: Vec<String> = assign.volunteer_ids.iter()
                    .map(|vid| self.config.volunteers.iter().find(|v| v.id == *vid).map_or(vid.clone(), |v| v.name.clone()))
                    .collect();
                let vol_names = vols.join("; ");

                csv.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
                    day, start, end, div_name, act_label, teams, field_name, vol_names
                ));
            }

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
