use eframe::egui;
use crate::core::{Clip, AppConfig};
use crate::video::{VideoPreview, WaveformData};
use crate::hotkeys::{HotkeyManager, HotkeyEvent};
use std::collections::HashMap;
use tokio::sync::broadcast;

pub struct ClipHelperApp {
    pub config: AppConfig,
    pub clips: Vec<Clip>,
    pub selected_clip_index: Option<usize>,
    pub video_preview: Option<VideoPreview>,
    pub waveforms: HashMap<String, WaveformData>,
    pub hotkey_receiver: broadcast::Receiver<HotkeyEvent>,
    pub new_clip_name: String,
}

impl ClipHelperApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        let config = AppConfig::load()?;
        config.ensure_directories()?;

        // Set up hotkeys
        let (hotkey_manager, hotkey_receiver) = HotkeyManager::new()?;
        
        // Store hotkey manager in a way that keeps it alive
        // This is a simplified version - in practice you'd want better lifecycle management
        std::thread::spawn(move || {
            loop {
                hotkey_manager.process_events();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        Ok(Self {
            config,
            clips: Vec::new(),
            selected_clip_index: None,
            video_preview: None,
            waveforms: HashMap::new(),
            hotkey_receiver,
            new_clip_name: String::new(),
        })
    }

    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
    }

    pub fn get_selected_clip(&self) -> Option<&Clip> {
        self.selected_clip_index.and_then(|i| self.clips.get(i))
    }

    pub fn get_selected_clip_mut(&mut self) -> Option<&mut Clip> {
        self.selected_clip_index.and_then(move |i| self.clips.get_mut(i))
    }

    pub fn select_clip(&mut self, index: usize) {
        if index < self.clips.len() {
            self.selected_clip_index = Some(index);
            
            // Initialize video preview for selected clip
            if let Some(clip) = self.clips.get(index) {
                self.video_preview = Some(VideoPreview::new(clip.trim_end));
            }
        }
    }

    pub fn delete_selected_clip(&mut self) -> anyhow::Result<()> {
        if let Some(index) = self.selected_clip_index {
            if let Some(clip) = self.clips.get_mut(index) {
                clip.is_deleted = true;
                
                // Move file to deleted directory
                let deleted_path = self.config.deleted_directory.join(
                    clip.original_file.file_name().unwrap()
                );
                std::fs::rename(&clip.original_file, &deleted_path)?;
            }
        }
        Ok(())
    }

    pub fn apply_trim(&mut self, force_overwrite: bool) -> anyhow::Result<()> {
        if let Some(index) = self.selected_clip_index {
            if let Some(clip) = self.clips.get_mut(index) {
                let output_filename = format!("{}.mkv", clip.get_output_filename());
                let output_path = self.config.trimmed_directory.join(output_filename);
                
                crate::video::VideoProcessor::trim_clip(clip, &output_path, force_overwrite)?;
                clip.is_trimmed = true;
            }
        }
        Ok(())
    }

    fn process_hotkey_events(&mut self) {
        while let Ok(event) = self.hotkey_receiver.try_recv() {
            match event {
                HotkeyEvent::ClipRequested(duration) => {
                    log::info!("Hotkey triggered for {:?}", duration);
                    // TODO: Find matching replay file and add clip
                    // This would involve checking the replay directory for files
                    // that match the current timestamp within the 10-second window
                }
            }
        }
    }
}

impl eframe::App for ClipHelperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_hotkey_events();
        
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Settings").clicked() {
                        // TODO: Open settings dialog
                    }
                    if ui.button("Exit").clicked() {
                        std::process::exit(0);
                    }
                });
                
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        // TODO: Show about dialog
                    }
                });
            });
        });

        egui::SidePanel::left("clip_list").show(ctx, |ui| {
            self.show_clip_list(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(selected_index) = self.selected_clip_index {
                if selected_index < self.clips.len() {
                    self.show_clip_editor(ui);
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a clip to edit");
                });
            }
        });

        // Request repaint to handle continuous updates
        ctx.request_repaint();
    }
}

impl ClipHelperApp {
    fn show_clip_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clips");
        
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut selected_index = self.selected_clip_index;
            
            for (index, clip) in self.clips.iter().enumerate() {
                let is_selected = selected_index == Some(index);
                
                if ui.selectable_label(is_selected, &clip.get_output_filename()).clicked() {
                    selected_index = Some(index);
                }
            }
            
            if selected_index != self.selected_clip_index {
                if let Some(index) = selected_index {
                    self.select_clip(index);
                }
            }
        });
    }

    fn show_clip_editor(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clip Editor");
        
        // Clip name input
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.new_clip_name);
        });
        
        // Timeline would go here
        self.show_timeline(ui);
        
        // Control buttons
        self.show_controls(ui);
        
        // Audio track controls
        self.show_audio_controls(ui);
        
        // Action buttons
        ui.horizontal(|ui| {
            if ui.button("Apply Trim").clicked() {
                if let Err(e) = self.apply_trim(false) {
                    log::error!("Failed to apply trim: {}", e);
                }
            }
            
            if ui.button("Delete").clicked() {
                if let Err(e) = self.delete_selected_clip() {
                    log::error!("Failed to delete clip: {}", e);
                }
            }
        });
    }

    fn show_timeline(&mut self, ui: &mut egui::Ui) {
        ui.label("Timeline (TODO: Implement timeline with scrubbing)");
        // TODO: Implement proper timeline widget
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("⏮ Start").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.goto_start();
                }
            }
            
            if ui.button("⏪ -10s").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_backward(10.0);
                }
            }
            
            if ui.button("⏪ -5s").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_backward(5.0);
                }
            }
            
            if ui.button("⏪ -3s").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_backward(3.0);
                }
            }
            
            if let Some(preview) = &mut self.video_preview {
                if ui.button(if preview.is_playing { "⏸" } else { "▶" }).clicked() {
                    preview.toggle_playback();
                }
            }
            
            if ui.button("3s ⏩").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_forward(3.0);
                }
            }
            
            if ui.button("5s ⏩").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_forward(5.0);
                }
            }
            
            if ui.button("10s ⏩").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.skip_forward(10.0);
                }
            }
            
            if ui.button("Last 5s ⏭").clicked() {
                if let Some(preview) = &mut self.video_preview {
                    preview.goto_last_5_seconds();
                }
            }
        });
        
        // Trim controls
        ui.horizontal(|ui| {
            ui.label("Start:");
            if ui.button("-5s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_start = (clip.trim_start - 5.0).max(0.0);
                }
            }
            if ui.button("-1s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_start = (clip.trim_start - 1.0).max(0.0);
                }
            }
            if ui.button("+1s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_start = (clip.trim_start + 1.0).min(clip.trim_end - 0.1);
                }
            }
            if ui.button("+5s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_start = (clip.trim_start + 5.0).min(clip.trim_end - 0.1);
                }
            }
        });
        
        ui.horizontal(|ui| {
            ui.label("End:");
            if ui.button("-5s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_end = (clip.trim_end - 5.0).max(clip.trim_start + 0.1);
                }
            }
            if ui.button("-1s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_end = (clip.trim_end - 1.0).max(clip.trim_start + 0.1);
                }
            }
            if ui.button("+1s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_end = (clip.trim_end + 1.0).min(clip.duration_seconds as f64);
                }
            }
            if ui.button("+5s").clicked() {
                if let Some(clip) = self.get_selected_clip_mut() {
                    clip.trim_end = (clip.trim_end + 5.0).min(clip.duration_seconds as f64);
                }
            }
        });
    }

    fn show_audio_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Audio Tracks");
        
        if let Some(clip) = self.get_selected_clip_mut() {
            for track in &mut clip.audio_tracks {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut track.enabled, &track.name);
                    ui.checkbox(&mut track.surround_mode, "Surround L/R");
                });
            }
        }
    }
}
