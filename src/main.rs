#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gui;
mod model;
mod scheduler;
mod validator;

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
            use eframe::egui::{FontData, FontDefinitions, FontFamily};

            let mut fonts = FontDefinitions::default();

            // Primary UI typeface (Lato) plus a bold cut used for headings, and
            // the Noto symbol fonts as a glyph fallback for emoji/icons.
            fonts.font_data.insert(
                "Lato".to_owned(),
                FontData::from_static(include_bytes!("../assets/Lato-Regular.ttf")),
            );
            fonts.font_data.insert(
                "LatoBold".to_owned(),
                FontData::from_static(include_bytes!("../assets/Lato-Bold.ttf")),
            );
            fonts.font_data.insert(
                "NotoSansSymbols".to_owned(),
                FontData::from_static(include_bytes!("../assets/NotoSansSymbols-Regular.ttf")),
            );
            fonts.font_data.insert(
                "NotoSansSymbols2".to_owned(),
                FontData::from_static(include_bytes!("../assets/NotoSansSymbols2-Regular.ttf")),
            );

            // Lato leads the proportional stack; symbols fall in behind it so any
            // glyph Lato lacks (emoji, dingbats) is still rendered.
            let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
            proportional.insert(0, "Lato".to_owned());
            proportional.push("NotoSansSymbols".to_owned());
            proportional.push("NotoSansSymbols2".to_owned());

            let mono = fonts.families.entry(FontFamily::Monospace).or_default();
            mono.push("NotoSansSymbols".to_owned());
            mono.push("NotoSansSymbols2".to_owned());

            // A dedicated bold family that headings map to via the text-style scale.
            fonts.families.insert(
                FontFamily::Name("Heading".into()),
                vec![
                    "LatoBold".to_owned(),
                    "NotoSansSymbols".to_owned(),
                    "NotoSansSymbols2".to_owned(),
                ],
            );

            cc.egui_ctx.set_fonts(fonts);
            // Build the app first: AppState::default() loads + activates the
            // colour theme, so styling must happen after it to pick that up.
            let app = gui::AppState::default();
            gui::setup_custom_style(&cc.egui_ctx);
            Box::new(app)
        }),
    )
}
