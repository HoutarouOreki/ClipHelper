mod core;
mod gui;
mod hotkeys;
mod video;

use eframe::egui;
use gui::ClipHelperApp;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Clip Helper - OBS Replay Buffer Trimmer"),
        ..Default::default()
    };

    eframe::run_native(
        "Clip Helper",
        options,
        Box::new(|cc| {
            match ClipHelperApp::new(cc) {
                Ok(app) => Ok(Box::new(app)),
                Err(e) => {
                    eprintln!("Failed to initialize app: {}", e);
                    std::process::exit(1);
                }
            }
        }),
    ).map_err(|e| anyhow::anyhow!("Failed to run app: {}", e))?;

    Ok(())
}
