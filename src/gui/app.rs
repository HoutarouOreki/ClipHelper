use eframe::egui;
use crate::core::{Clip, AppConfig, FileMonitor, NewReplayFile};
use crate::video::{VideoPreview, WaveformData};
use crate::hotkeys::{HotkeyManager, HotkeyEvent};
use crate::gui::timeline::TimelineWidget;
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
    pub show_directory_dialog: bool,
    pub status_message: String,
    pub directory_browser_path: std::path::PathBuf,
    pub timeline_widget: TimelineWidget,
    pub show_drives_view: bool,
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
            show_directory_dialog: false,
            status_message: String::new(),
            directory_browser_path: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("C:\\")),
            timeline_widget: TimelineWidget::new(),
            show_drives_view: false,
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
                    if ui.button("Select OBS Replay Directory").clicked() {
                        self.show_directory_dialog = true;
                        ui.close_menu();
                    }
                    
                    ui.separator();
                    
                    if ui.button("Settings").clicked() {
                        // TODO: Open settings dialog
                        ui.close_menu();
                    }
                    if ui.button("Exit").clicked() {
                        std::process::exit(0);
                    }
                });
                
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        // TODO: Show about dialog
                        ui.close_menu();
                    }
                });
                
                // Show current directory status
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(ref dir) = self.watched_directory {
                        ui.label(format!("📁 {}", dir.file_name().unwrap_or_default().to_string_lossy()));
                    } else {
                        ui.label("❌ No directory selected");
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
                    if self.watched_directory.is_some() {
                        ui.label("Select a clip to edit");
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.heading("Welcome to ClipHelper");
                            ui.label("To get started, select your OBS replay directory from the File menu.");
                            ui.add_space(20.0);
                            if ui.button("📁 Select OBS Replay Directory").clicked() {
                                self.show_directory_dialog = true;
                            }
                        });
                    }
                });
            }
        });

        // Show directory selection dialog
        if self.show_directory_dialog {
            self.show_directory_selection_dialog(ctx);
        }

        // Status bar at bottom
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                if self.status_message.is_empty() {
                    ui.label("Ready");
                } else {
                    ui.label(&self.status_message);
                }
                
                // Hotkey status
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("Hotkeys: Ctrl+Numpad1-5 (15s/30s/1m/2m/5m)");
                });
            });
        });

        // Request repaint to handle continuous updates
        ctx.request_repaint();
    }
}

impl ClipHelperApp {
    fn show_clip_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clips");
        
        // Status message if no directory selected
        if self.watched_directory.is_none() {
            ui.label("❌ No directory selected");
            ui.small("Select an OBS replay directory from the File menu");
            return;
        }
        
        // Show directory status
        if let Some(ref dir) = self.watched_directory {
            ui.small(format!("📁 {}", dir.file_name().unwrap_or_default().to_string_lossy()));
        }
        
        ui.separator();
        
        // Button to scan for existing files
        if ui.button("🔄 Scan for Replay Files").clicked() {
            self.scan_and_load_replay_files();
        }
        
        ui.separator();
        
        // Show clips
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.clips.is_empty() {
                ui.label("No clips loaded");
                ui.small("Press the scan button above to load existing replay files");
                ui.small("Or trigger a hotkey to capture new clips");
            } else {
                let mut selected_index = self.selected_clip_index;
                
                for (index, clip) in self.clips.iter().enumerate() {
                    let is_selected = selected_index == Some(index);
                    
                    if ui.selectable_label(is_selected, &clip.get_output_filename()).clicked() {
                        selected_index = Some(index);
                    }
                    
                    // Show clip duration
                    ui.small(format!("Duration: {:.1}s", clip.duration_seconds));
                }
                
                if selected_index != self.selected_clip_index {
                    if let Some(index) = selected_index {
                        self.select_clip(index);
                    }
                }
            }
        });
    }

    fn scan_and_load_replay_files(&mut self) {
        if let Some(ref watched_dir) = self.watched_directory {
            log::info!("Scanning for existing replay files in: {}", watched_dir.display());
            
            match FileMonitor::scan_existing_files(watched_dir) {
                Ok(existing_files) => {
                    log::info!("Found {} existing replay files", existing_files.len());
                    
                    // Clear existing clips first
                    self.clips.clear();
                    self.selected_clip_index = None;
                    
                    // Create clips for found files (limit to recent 20 files)
                    for file in existing_files.into_iter().take(20) {
                        // Default to 30-second clips for existing files
                        let file_path = file.path.clone();
                        match Clip::new(file.path, crate::core::ClipDuration::Seconds30) {
                            Ok(clip) => {
                                log::debug!("Loaded existing file: {}", clip.get_output_filename());
                                self.clips.push(clip);
                            }
                            Err(e) => {
                                log::error!("Failed to create clip for existing file {:?}: {}", file_path, e);
                            }
                        }
                    }
                    
                    self.status_message = format!("Loaded {} replay files", self.clips.len());
                    log::info!("Successfully loaded {} clips from existing files", self.clips.len());
                }
                Err(e) => {
                    log::error!("Failed to scan existing files: {}", e);
                    self.status_message = format!("Error scanning files: {}", e);
                }
            }
        }
    }

    fn show_clip_editor(&mut self, ui: &mut egui::Ui) {
        if let Some(selected_index) = self.selected_clip_index {
            if let Some(clip) = self.clips.get(selected_index) {
                ui.heading("Clip Editor");
                
                // Clip info
                ui.horizontal(|ui| {
                    ui.label("File:");
                    ui.label(clip.original_file.file_name().unwrap_or_default().to_string_lossy());
                });
                
                ui.horizontal(|ui| {
                    ui.label("Duration:");
                    ui.label(format!("{:.1}s", clip.duration_seconds));
                    ui.separator();
                    ui.label("Trim:");
                    ui.label(format!("{:.1}s - {:.1}s", clip.trim_start, clip.trim_end));
                });
                
                // Clip name input
                ui.horizontal(|ui| {
                    ui.label("Output name:");
                    ui.text_edit_singleline(&mut self.new_clip_name);
                });
                
                ui.separator();
                
                // Timeline would go here
                self.show_timeline(ui);
                
                ui.separator();
                
                // Control buttons
                self.show_controls(ui);
                
                ui.separator();
                
                // Audio track controls
                self.show_audio_controls(ui);
                
                ui.separator();
                
                // Action buttons
                ui.horizontal(|ui| {
                    if ui.button("✂️ Apply Trim").clicked() {
                        if let Err(e) = self.apply_trim(false) {
                            log::error!("Failed to apply trim: {}", e);
                            self.status_message = format!("Error applying trim: {}", e);
                        } else {
                            self.status_message = "Trim applied successfully".to_string();
                        }
                    }
                    
                    if ui.button("🗑️ Delete").clicked() {
                        if let Err(e) = self.delete_selected_clip() {
                            log::error!("Failed to delete clip: {}", e);
                            self.status_message = format!("Error deleting clip: {}", e);
                        } else {
                            self.status_message = "Clip moved to deleted folder".to_string();
                        }
                    }
                    
                    // Shift+click for force overwrite
                    ui.separator();
                    ui.label("Hold Shift and click Apply to overwrite existing files");
                });
            }
        }
    }

    fn show_timeline(&mut self, ui: &mut egui::Ui) {
        if let Some(selected_index) = self.selected_clip_index {
            if let Some(clip) = self.clips.get_mut(selected_index) {
                self.timeline_widget.show(ui, clip, &mut self.video_preview);
            }
        } else {
            ui.label("No clip selected");
        }
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

    fn show_directory_selection_dialog(&mut self, ctx: &egui::Context) {
        egui::Window::new("Select OBS Replay Directory")
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                ui.label("Choose the directory where OBS saves your replay files:");
                
                // Current path display
                ui.horizontal(|ui| {
                    ui.label("Current path:");
                    if self.show_drives_view {
                        ui.text_edit_singleline(&mut "This PC".to_string());
                    } else {
                        ui.text_edit_singleline(&mut format!("{}", self.directory_browser_path.display()));
                    }
                });
                
                ui.separator();
                
                // Navigation buttons
                ui.horizontal(|ui| {
                    if self.show_drives_view {
                        // When showing drives, only show back button if we came from a directory
                        if ui.button("📁 Browse Directories").clicked() {
                            self.show_drives_view = false;
                        }
                    } else {
                        if ui.button("⬆ Parent Directory").clicked() {
                            if let Some(parent) = self.directory_browser_path.parent() {
                                self.directory_browser_path = parent.to_path_buf();
                            }
                        }
                        
                        if ui.button("💻 This PC").clicked() {
                            self.show_drives_view = true;
                        }
                    }
                });
                
                ui.separator();
                
                // Directory listing
                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        if self.show_drives_view {
                            // Show available drives
                            ui.label("Available Drives:");
                            ui.separator();
                            
                            let mut found_drives = false;
                            for drive_letter in 'A'..='Z' {
                                let drive_path = format!("{}:\\", drive_letter);
                                let drive_pathbuf = std::path::PathBuf::from(&drive_path);
                                
                                // Check if drive exists by trying to read the root directory
                                if drive_pathbuf.exists() && std::fs::read_dir(&drive_pathbuf).is_ok() {
                                    found_drives = true;
                                    let drive_name = get_drive_label(&drive_pathbuf).unwrap_or_else(|| {
                                        format!("Local Disk ({}:)", drive_letter)
                                    });
                                    
                                    if ui.selectable_label(false, format!("💽 {} ({}:)", drive_name, drive_letter)).clicked() {
                                        self.directory_browser_path = drive_pathbuf;
                                        self.show_drives_view = false;
                                    }
                                }
                            }
                            
                            if !found_drives {
                                ui.label("❌ No accessible drives found");
                            }
                        } else {
                            // Show directory contents
                            if let Ok(entries) = std::fs::read_dir(&self.directory_browser_path) {
                                let mut dirs: Vec<_> = entries
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.path().is_dir())
                                    .collect();
                                dirs.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
                                
                                for entry in dirs {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    if ui.selectable_label(false, format!("📁 {}", name)).clicked() {
                                        self.directory_browser_path = entry.path();
                                        self.show_drives_view = false;
                                    }
                                }
                            } else {
                                ui.label("❌ Unable to read directory");
                            }
                        }
                    });
                
                ui.separator();
                
                // Action buttons
                ui.horizontal(|ui| {
                    if ui.button("✅ Select This Directory").clicked() {
                        self.set_watched_directory(self.directory_browser_path.clone());
                        self.show_directory_dialog = false;
                    }
                    
                    if ui.button("❌ Cancel").clicked() {
                        self.show_directory_dialog = false;
                    }
                });
            });
    }

    fn set_watched_directory(&mut self, path: std::path::PathBuf) {
        log::info!("Setting watched directory to: {}", path.display());
        
        // Stop existing file monitoring
        self.file_monitor = None;
        self.file_receiver = None;
        
        // Start new file monitoring
        match FileMonitor::new(&path) {
            Ok((monitor, receiver)) => {
                self.file_monitor = Some(monitor);
                self.file_receiver = Some(receiver);
                self.watched_directory = Some(path.clone());
                
                // Update config and save
                self.config.last_watched_directory = Some(path.clone());
                if let Err(e) = self.config.save() {
                    log::error!("Failed to save config: {}", e);
                } else {
                    log::info!("Config saved with new watched directory");
                }
                
                // Update directory paths in config
                self.config.obs_replay_directory = path.clone();
                self.config.deleted_directory = path.join("deleted");
                self.config.trimmed_directory = path.join("trimmed");
                
                // Ensure directories exist
                if let Err(e) = self.config.ensure_directories() {
                    log::error!("Failed to ensure directories: {}", e);
                }
                
                // Load existing clips
                self.load_existing_clips();
                
                self.status_message = format!("Successfully set directory: {}", path.display());
                log::info!("File monitoring started for directory: {}", path.display());
            }
            Err(e) => {
                log::error!("Failed to start file monitoring: {}", e);
                self.status_message = format!("Failed to monitor directory: {}", e);
            }
        }
    }
}

// Helper function to get drive labels on Windows
fn get_drive_label(drive_path: &std::path::Path) -> Option<String> {
    // For now, return a simple default name
    // On Windows, you could use Windows API to get the actual volume label
    // but for simplicity, we'll use generic names
    if let Some(drive_str) = drive_path.to_str() {
        match &drive_str[..1] {
            "C" => Some("Windows (C:)".to_string()),
            "D" => Some("Data (D:)".to_string()),
            "E" => Some("External (E:)".to_string()),
            "F" => Some("Drive (F:)".to_string()),
            _ => Some(format!("Local Disk ({}:)", &drive_str[..1])),
        }
    } else {
        None
    }
}
