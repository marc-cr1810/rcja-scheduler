#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod model;
mod scheduler;
mod validator;
mod gui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([1000.0, 650.0])
            .with_title("RoboCup Jr Australia Tournament Scheduler"),
        ..Default::default()
    };

    eframe::run_native(
        "rcja_tournament_scheduler",
        options,
        Box::new(|cc| {
            let mut fonts = eframe::egui::FontDefinitions::default();

            fonts.font_data.insert(
                "NotoSansSymbols".to_owned(),
                eframe::egui::FontData::from_static(
                    include_bytes!("../assets/NotoSansSymbols-Regular.ttf")
                ),
            );
            fonts.font_data.insert(
                "NotoSansSymbols2".to_owned(),
                eframe::egui::FontData::from_static(
                    include_bytes!("../assets/NotoSansSymbols2-Regular.ttf")
                ),
            );

            for family in [
                eframe::egui::FontFamily::Proportional,
                eframe::egui::FontFamily::Monospace,
            ] {
                let list = fonts.families.entry(family).or_default();
                list.push("NotoSansSymbols".to_owned());
                list.push("NotoSansSymbols2".to_owned());
            }

            cc.egui_ctx.set_fonts(fonts);
            gui::setup_custom_style(&cc.egui_ctx);
            Box::new(gui::AppState::default())
        }),
    )
}
