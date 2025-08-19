use eframe::egui;
use crate::core::{Clip, AppConfig, FileMonitor, NewReplayFile};
use crate::video::{VideoPreview, WaveformData};
use crate::hotkeys::{HotkeyManager, HotkeyEvent};
use std::collections::HashMap;
use tokio::sync::broadcast;
use chrono::Utc;

pub struct ClipHelperApp {
    pub config: AppConfig,
    pub clips: Vec<Clip>,
    pub selected_clip_index: Option<usize>,
    pub video_preview: Option<VideoPreview>,
    pub waveforms: HashMap<String, WaveformData>,
    pub hotkey_receiver: broadcast::Receiver<HotkeyEvent>,
    pub file_monitor: Option<FileMonitor>,
    pub file_receiver: Option<broadcast::Receiver<NewReplayFile>>,
    pub new_clip_name: String,
    pub pending_clip_requests: Vec<(chrono::DateTime<Utc>, crate::core::ClipDuration)>,
    pub watched_directory: Option<std::path::PathBuf>,
}

impl ClipHelperApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        let config = AppConfig::load()?;
        config.ensure_directories()?;

        // Set up hotkeys
        let (hotkey_manager, hotkey_receiver) = HotkeyManager::new(&config)?;
        
        // Store hotkey manager in a way that keeps it alive
        // This is a simplified version - in practice you'd want better lifecycle management
        log::info!("Starting hotkey processing thread...");
        std::thread::spawn(move || {
            let mut iteration = 0;
            loop {
                hotkey_manager.process_events();
                iteration += 1;
                
                // Log heartbeat every 10 seconds (1000 iterations * 10ms)
                if iteration % 1000 == 0 {
                    log::debug!("Hotkey processing thread alive (iteration {})", iteration);
                }
                
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        // Initialize file monitoring if we have a last watched directory
        let (file_monitor, file_receiver, watched_directory) = if let Some(ref last_dir) = config.last_watched_directory {
            if last_dir.exists() {
                log::info!("Restoring last watched directory: {}", last_dir.display());
                match FileMonitor::new(last_dir) {
                    Ok((monitor, receiver)) => {
                        log::info!("File monitoring initialized for {}", last_dir.display());
                        (Some(monitor), Some(receiver), Some(last_dir.clone()))
                    }
                    Err(e) => {
                        log::error!("Failed to initialize file monitoring for {}: {}", last_dir.display(), e);
                        (None, None, None)
                    }
                }
            } else {
                log::warn!("Last watched directory no longer exists: {}", last_dir.display());
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        Ok(Self {
            config,
            clips: Vec::new(),
            selected_clip_index: None,
            video_preview: None,
            waveforms: HashMap::new(),
            hotkey_receiver,
            file_monitor,
            file_receiver,
            new_clip_name: String::new(),
            pending_clip_requests: Vec::new(),
            watched_directory,
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
                    clip.original_file.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unknown_file")
                );
                
                log::info!("Moving file to deleted directory: {} -> {}", 
                    clip.original_file.display(), deleted_path.display());
                
                if let Err(e) = std::fs::rename(&clip.original_file, &deleted_path) {
                    log::error!("Failed to move file to deleted directory: {}", e);
                    return Err(anyhow::anyhow!("Failed to move file to deleted directory: {}", e));
                }
                
                log::info!("File successfully moved to deleted directory");
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
                    let now = Utc::now();
                    log::info!("Hotkey triggered for {:?} at {}", duration, now);
                    
                    // Store the clip request with timestamp
                    self.pending_clip_requests.push((now, duration.clone()));
                    
                    // Try to match with recent files
                    self.try_match_clip_request(now, duration);
                }
            }
        }
    }
    
    fn process_file_events(&mut self) {
        // Collect new files first
        let mut new_files = Vec::new();
        if let Some(ref mut receiver) = self.file_receiver {
            while let Ok(new_file) = receiver.try_recv() {
                log::info!("New file detected: {:?}", new_file.path);
                new_files.push(new_file);
            }
        }
        
        // Process each new file
        for new_file in new_files {
            // Check if this file matches any pending clip requests
            let mut matched_requests = Vec::new();
            
            for (i, (request_time, duration)) in self.pending_clip_requests.iter().enumerate() {
                if Self::timestamps_match_static(*request_time, new_file.timestamp) {
                    matched_requests.push((i, new_file.clone(), duration.clone()));
                }
            }
            
            // Process matched requests
            for (index, file, duration) in matched_requests.iter().rev() {
                self.create_clip_from_file(file.clone(), duration.clone());
                self.pending_clip_requests.remove(*index);
            }
        }
    }
    
    fn try_match_clip_request(&mut self, request_time: chrono::DateTime<Utc>, duration: crate::core::ClipDuration) {
        if let Some(ref watched_dir) = self.watched_directory {
            // Scan for existing files that might match
            if let Ok(existing_files) = FileMonitor::scan_existing_files(watched_dir) {
                for file in existing_files {
                    if self.timestamps_match(request_time, file.timestamp) {
                        self.create_clip_from_file(file, duration);
                        // Remove the pending request
                        self.pending_clip_requests.retain(|(time, _)| *time != request_time);
                        return;
                    }
                }
            }
        }
        
        // Keep the request pending for a bit in case the file appears later
        // Remove old pending requests (older than 30 seconds)
        let cutoff = Utc::now() - chrono::Duration::seconds(30);
        self.pending_clip_requests.retain(|(time, _)| *time > cutoff);
    }
    
    fn try_match_file_to_requests(&mut self, new_file: &NewReplayFile) {
        let mut clips_to_create = Vec::new();
        let mut indices_to_remove = Vec::new();
        
        for (i, (request_time, duration)) in self.pending_clip_requests.iter().enumerate() {
            if Self::timestamps_match_static(*request_time, new_file.timestamp) {
                clips_to_create.push((new_file.clone(), duration.clone()));
                indices_to_remove.push(i);
            }
        }
        
        // Remove matched requests (in reverse order to maintain indices)
        for &index in indices_to_remove.iter().rev() {
            self.pending_clip_requests.remove(index);
        }
        
        // Create clips
        for (file, duration) in clips_to_create {
            self.create_clip_from_file(file, duration);
        }
    }
    
    fn timestamps_match(&self, request_time: chrono::DateTime<Utc>, file_time: chrono::DateTime<Utc>) -> bool {
        Self::timestamps_match_static(request_time, file_time)
    }
    
    fn timestamps_match_static(request_time: chrono::DateTime<Utc>, file_time: chrono::DateTime<Utc>) -> bool {
        let diff = (request_time - file_time).num_seconds().abs();
        diff <= 10 // Within 10 seconds
    }
    
    fn create_clip_from_file(&mut self, file: NewReplayFile, duration: crate::core::ClipDuration) {
        match Clip::new(file.path, duration) {
            Ok(clip) => {
                log::info!("Created clip: {}", clip.get_output_filename());
                self.clips.push(clip);
            }
            Err(e) => {
                log::error!("Failed to create clip: {}", e);
            }
        }
    }
    
    fn load_existing_clips(&mut self) {
        if let Some(ref watched_dir) = self.watched_directory {
            if let Ok(existing_files) = FileMonitor::scan_existing_files(watched_dir) {
                log::info!("Found {} existing replay files", existing_files.len());
                // For now, just log them. Later we could offer to import them.
                for file in existing_files.iter().take(10) { // Show first 10
                    log::info!("Existing file: {:?}", file.path.file_name());
                }
            }
        }
    }


}

impl eframe::App for ClipHelperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process events
        self.process_hotkey_events();
        self.process_file_events();
        
        // Periodic cleanup of old clip requests
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(30);
        self.pending_clip_requests.retain(|(time, _)| *time > cutoff);
        
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
