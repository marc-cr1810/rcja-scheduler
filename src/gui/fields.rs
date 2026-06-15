use crate::gui::AppState;
use crate::gui::theme;
use crate::model::{Field, FieldKind};
use crate::scheduler::sanitize_name;
use eframe::egui::{self, RichText};

impl AppState {
    pub(super) fn draw_fields(&mut self, ui: &mut egui::Ui) {
        ui.heading("Competition Fields & Arenas");
        ui.label("Manage the physical locations where matches and runs take place.");
        ui.add_space(10.0);

        // Add Competition Field
        ui.horizontal(|ui| {
            ui.label("New Field Name:");
            let res = ui.text_edit_singleline(&mut self.new_field_name);
            if (ui.button("+ Add Field").clicked() || (res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && !self.new_field_name.trim().is_empty() {
                    let existing_ids: Vec<String> = self.config.fields.iter().map(|f| f.id.clone()).collect();
                    let id = crate::scheduler::unique_id(&sanitize_name(&self.new_field_name), &existing_ids);
                    self.config.fields.push(Field {
                        id,
                        name: self.new_field_name.clone(),
                        kind: FieldKind::Competition,
                        allowed_divisions: None,
                    });
                    self.new_field_name.clear();
                    self.clear_schedule();
                    self.update_diagnostics();
                    self.status_message = "Competition field added!".to_string();
                }
        });

        ui.add_space(15.0);

        // Clone the divisions list to prevent borrow-checker violations
        let divisions_list: Vec<(String, String)> = self.config.divisions.iter()
            .map(|d| (d.id.clone(), d.name.clone()))
            .collect();

        let mut to_remove = None;
        let mut fields_changed = false;

        // Competition Fields Table
        egui::Grid::new("comp_fields_grid").num_columns(3).spacing(egui::vec2(20.0, 10.0)).striped(true).show(ui, |ui| {
            ui.label(RichText::new("Field/Arena Name").strong());
            ui.label(RichText::new("Allowed Divisions (Restrictive)").strong());
            ui.label(RichText::new("Actions").strong());
            ui.end_row();

            for (idx, field) in self.config.fields.iter_mut().enumerate() {
                if field.kind != FieldKind::Competition {
                    continue;
                }

                if ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut field.name)).changed() {
                    fields_changed = true;
                }

                ui.horizontal(|ui| {
                    let mut is_restricted = field.allowed_divisions.is_some();
                    if ui.checkbox(&mut is_restricted, "Restrict?").changed() {
                        if is_restricted {
                            field.allowed_divisions = Some(Vec::new());
                        } else {
                            field.allowed_divisions = None;
                        }
                        fields_changed = true;
                    }

                    if let Some(ref mut allowed) = field.allowed_divisions {
                        for (div_id, div_name) in &divisions_list {
                            let mut has_div = allowed.contains(div_id);
                            if ui.checkbox(&mut has_div, div_name).changed() {
                                if has_div {
                                    allowed.push(div_id.clone());
                                } else {
                                    allowed.retain(|x| x != div_id);
                                }
                                fields_changed = true;
                            }
                        }
                    } else {
                        ui.label(RichText::new("Allows All Divisions").color(theme::TEXT_MUTED));
                    }
                });

                if ui.button("🗑 Delete").clicked() {
                    to_remove = Some(idx);
                }

                ui.end_row();
            }
        });

        ui.add_space(30.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("Interview Tables");
        ui.label("Manage the tables where team interviews are conducted.");
        ui.add_space(10.0);

        // Add Interview Table
        ui.horizontal(|ui| {
            ui.label("New Table Name:");
            let res = ui.text_edit_singleline(&mut self.new_table_name);
            if (ui.button("+ Add Table").clicked() || (res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && !self.new_table_name.trim().is_empty() {
                    let existing_ids: Vec<String> = self.config.fields.iter().map(|f| f.id.clone()).collect();
                    let id = crate::scheduler::unique_id(&sanitize_name(&self.new_table_name), &existing_ids);
                    self.config.fields.push(Field {
                        id,
                        name: self.new_table_name.clone(),
                        kind: FieldKind::Interview,
                        allowed_divisions: None, // Interviews are currently open to all divisions with interviews enabled
                    });
                    self.new_table_name.clear();
                    self.clear_schedule();
                    self.update_diagnostics();
                    self.status_message = "Interview table added!".to_string();
                }
        });

        ui.add_space(15.0);

        // Interview Tables Table
        egui::Grid::new("interview_tables_grid").num_columns(2).spacing(egui::vec2(20.0, 10.0)).striped(true).show(ui, |ui| {
            ui.label(RichText::new("Table Name").strong());
            ui.label(RichText::new("Actions").strong());
            ui.end_row();

            for (idx, field) in self.config.fields.iter_mut().enumerate() {
                if field.kind != FieldKind::Interview {
                    continue;
                }

                if ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut field.name)).changed() {
                    fields_changed = true;
                }

                if ui.button("🗑 Delete").clicked() {
                    to_remove = Some(idx);
                }

                ui.end_row();
            }
        });

        if fields_changed {
            self.clear_schedule();
            self.update_diagnostics();
        }

        if let Some(idx) = to_remove {
            self.config.fields.remove(idx);
            self.clear_schedule();
            self.update_diagnostics();
            self.status_message = "Item deleted.".to_string();
        }
    }
}
