use eframe::egui;
use crate::core::{Clip, AppConfig, FileMonitor, NewReplayFile};
use crate::video::{VideoPreview, WaveformData};
use crate::hotkeys::{HotkeyManager, HotkeyEvent};
use crate::gui::timeline::TimelineWidget;
use std::collections::HashMap;
use tokio::sync::broadcast;
use chrono::Local;

#[derive(Debug, Clone)]
pub struct SessionGroup {
    pub date: String, // "2025-08-19"
    pub start_time: String, // "14:56"
    pub end_time: String, // "17:11"
    pub clips: Vec<usize>, // indices into the main clips vector
}

#[derive(Debug, Clone)]
pub struct PendingClipRequest {
    pub timestamp: chrono::DateTime<Local>,
    pub duration: crate::core::ClipDuration,
    pub created_at: std::time::Instant,
    pub last_retry: std::time::Instant,
    pub retry_count: u32,
}

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
    pub pending_clip_requests: Vec<PendingClipRequest>,
    pub watched_directory: Option<std::path::PathBuf>,
    pub show_directory_dialog: bool,
    pub status_message: String,
    pub directory_browser_path: std::path::PathBuf,
    pub timeline_widget: TimelineWidget,
    pub show_drives_view: bool,
    /// Last time we checked for video info updates (for clips that might still be writing)
    pub last_video_info_check: std::time::Instant,
    /// Whether we've done the initial file scan yet
    pub initial_scan_completed: bool,
}

impl ClipHelperApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        // Set global text color to white
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::WHITE);
        cc.egui_ctx.set_visuals(visuals);
        
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

        // Load existing clips from the watched directory (without blocking on video info)
        let clips = Vec::new();
        // Note: File scanning moved to background - UI shows immediately

        Ok(Self {
            config,
            clips,
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
            last_video_info_check: std::time::Instant::now(),
            initial_scan_completed: false,
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
            
            // Lazily load video info for the selected clip if not already loaded
            if let Some(clip) = self.clips.get_mut(index) {
                if clip.video_length_seconds.is_none() {
                    log::debug!("Loading video info for selected clip: {}", clip.get_output_filename());
                    match clip.populate_video_info() {
                        Ok(is_valid) => {
                            if is_valid {
                                log::debug!("Video info loaded successfully for {}", clip.get_output_filename());
                            } else {
                                log::debug!("Video info loaded but file is still being written: {}", clip.get_output_filename());
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to get video info for {}: {}", clip.get_output_filename(), e);
                        }
                    }
                }
            }
            
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
                    let now = Local::now();
                    log::info!("Hotkey triggered for {:?} at {}", duration, now);
                    
                    // First, try to find existing clips that match this timestamp
                    // This includes clips that might be grayed out (still being written)
                    let mut found_existing = false;
                    log::debug!("Looking for clips matching timestamp: {}", now);
                    log::debug!("Current clips: {}", self.clips.len());
                    
                    for clip in self.clips.iter_mut() {
                        if clip.matches_timestamp(now) {
                            clip.set_target_duration(duration.clone());
                            found_existing = true;
                            
                            // If the clip needs video info update, try to update it now
                            if clip.needs_video_info_update() {
                                match clip.populate_video_info() {
                                    Ok(is_valid) => {
                                        if is_valid {
                                            log::info!("Video info updated after setting target duration for {}", clip.get_output_filename());
                                        }
                                    }
                                    Err(_) => {
                                        // File might still be writing, will be updated later
                                    }
                                }
                            }
                            break; // Only update the first matching clip
                        }
                    }
                    
                    // Always store the clip request for future file matching, even if we found existing clips
                    // This ensures that if OBS creates a new file shortly after the hotkey,
                    // it will get the correct target duration. This handles the case where:
                    // 1. User presses hotkey
                    // 2. OBS hasn't created the file yet (or file hasn't been detected)
                    // 3. File appears later and should still get the target duration applied
                    let now_instant = std::time::Instant::now();
                    self.pending_clip_requests.push(PendingClipRequest {
                        timestamp: now,
                        duration: duration.clone(),
                        created_at: now_instant,
                        last_retry: now_instant,
                        retry_count: 0,
                    });
                    
                    // If no existing clip matched, try to match with recent files
                    if !found_existing {
                        self.try_match_clip_request(now, duration);
                    }
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
            // First, check if this file matches any pending clip requests
            let mut matched_requests = Vec::new();
            
            for (i, request) in self.pending_clip_requests.iter().enumerate() {
                if Self::timestamps_match_static(request.timestamp, new_file.timestamp) {
                    matched_requests.push((i, new_file.clone(), request.duration.clone()));
                }
            }
            
            // Process matched requests
            for (index, file, duration) in matched_requests.iter().rev() {
                self.create_clip_from_file(file.clone(), Some(duration.clone()));
                self.pending_clip_requests.remove(*index);
            }
            
            // If no hotkey request matched, still add the file to the clip list automatically
            if matched_requests.is_empty() {
                self.create_clip_from_file(new_file, None);
            }
        }
    }
    
    fn try_match_clip_request(&mut self, request_time: chrono::DateTime<Local>, duration: crate::core::ClipDuration) {
        if let Some(ref watched_dir) = self.watched_directory {
            // Scan for existing files that might match
            if let Ok(existing_files) = FileMonitor::scan_existing_files(watched_dir) {
                for file in existing_files {
                    if self.timestamps_match(request_time, file.timestamp) {
                        self.create_clip_from_file(file, Some(duration));
                        // Remove the pending request
                        self.pending_clip_requests.retain(|req| req.timestamp != request_time);
                        return;
                    }
                }
            }
        }
        
        // Keep the request pending for a bit in case the file appears later
        // Remove old pending requests (older than 30 seconds)
        let cutoff = Local::now() - chrono::Duration::seconds(30);
        self.pending_clip_requests.retain(|req| req.timestamp > cutoff);
    }
    
    fn try_match_file_to_requests(&mut self, new_file: &NewReplayFile) {
        let mut clips_to_create = Vec::new();
        let mut indices_to_remove = Vec::new();
        
        for (i, request) in self.pending_clip_requests.iter().enumerate() {
            if Self::timestamps_match_static(request.timestamp, new_file.timestamp) {
                clips_to_create.push((new_file.clone(), request.duration.clone()));
                indices_to_remove.push(i);
            }
        }
        
        // Remove matched requests (in reverse order to maintain indices)
        for &index in indices_to_remove.iter().rev() {
            self.pending_clip_requests.remove(index);
        }
        
        // Create clips
        for (file, duration) in clips_to_create {
            self.create_clip_from_file(file, Some(duration));
        }
    }
    
    fn timestamps_match(&self, request_time: chrono::DateTime<Local>, file_time: chrono::DateTime<Local>) -> bool {
        Self::timestamps_match_static(request_time, file_time)
    }
    
    fn timestamps_match_static(request_time: chrono::DateTime<Local>, file_time: chrono::DateTime<Local>) -> bool {
        let diff = (request_time - file_time).num_seconds().abs();
        diff <= 10 // Within 10 seconds
    }
    
    fn create_clip_from_file(&mut self, file: NewReplayFile, duration: Option<crate::core::ClipDuration>) {
        let clip_result = if let Some(duration) = duration {
            // Create clip with specific target duration (from hotkey)
            Clip::new(file.path, duration)
        } else {
            // Create clip without target duration (auto-detected file)
            Clip::new_without_target(file.path)
        };
        
        match clip_result {
            Ok(clip) => {
                // Don't block on video info - load it lazily when needed
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
                // Files are logged during auto-refresh or manual scan
            }
        }
    }

    fn force_refresh_clips(&mut self) {
        // Force refresh regardless of current state
        if let Some(ref watched_dir) = self.watched_directory {
            log::debug!("Force refreshing clip list...");
            self.clips.clear(); // Clear existing clips
            
            match FileMonitor::scan_existing_files(watched_dir) {
                Ok(existing_files) => {
                    if !existing_files.is_empty() {
                        log::info!("Force refresh found {} files", existing_files.len());
                        
                        // Create clips for all found files
                        for file in existing_files {
                            let file_path = file.path.clone();
                            match Clip::new_without_target(file.path) {
                                Ok(clip) => {
                                    // Don't block on video info during refresh
                                    log::debug!("Force-loaded file: {}", clip.get_output_filename());
                                    self.clips.push(clip);
                                }
                                Err(e) => {
                                    log::error!("Failed to force-load clip for file {:?}: {}", file_path, e);
                                }
                            }
                        }
                        
                        self.status_message = format!("Refreshed {} clips", self.clips.len());
                    } else {
                        self.status_message = "No replay files found".to_string();
                    }
                }
                Err(e) => {
                    log::error!("Force refresh scan failed: {}", e);
                    self.status_message = format!("Refresh failed: {}", e);
                }
            }
        } else {
            self.status_message = "No directory being watched".to_string();
        }
    }

    fn group_clips_into_sessions(&self) -> Vec<SessionGroup> {
        if self.clips.is_empty() {
            return Vec::new();
        }

        let mut sessions = Vec::new();
        let mut current_session_clips = Vec::new();
        let mut session_start_time: Option<chrono::DateTime<Local>> = None;
        let mut last_clip_time: Option<chrono::DateTime<Local>> = None;

        // Sort clips by timestamp
        let mut sorted_indices: Vec<usize> = (0..self.clips.len()).collect();
        sorted_indices.sort_by(|&a, &b| self.clips[a].timestamp.cmp(&self.clips[b].timestamp));

        for &index in &sorted_indices {
            let clip = &self.clips[index];
            
            // Check if this clip starts a new session (gap > 1 hour)
            let starts_new_session = if let Some(last_time) = last_clip_time {
                let time_diff = clip.timestamp.signed_duration_since(last_time);
                time_diff.num_hours() >= 1
            } else {
                true // First clip always starts a new session
            };

            if starts_new_session && !current_session_clips.is_empty() {
                // Finish current session
                if let Some(start_time) = session_start_time {
                    if let Some(end_time) = last_clip_time {
                        let session = SessionGroup {
                            date: start_time.format("%Y-%m-%d").to_string(),
                            start_time: start_time.format("%H:%M").to_string(),
                            end_time: end_time.format("%H:%M").to_string(),
                            clips: current_session_clips.clone(),
                        };
                        sessions.push(session);
                    }
                }
                current_session_clips.clear();
            }

            // Start new session if needed
            if current_session_clips.is_empty() {
                session_start_time = Some(clip.timestamp);
            }

            current_session_clips.push(index);
            last_clip_time = Some(clip.timestamp);
        }

        // Add the last session
        if !current_session_clips.is_empty() {
            if let Some(start_time) = session_start_time {
                if let Some(end_time) = last_clip_time {
                    let session = SessionGroup {
                        date: start_time.format("%Y-%m-%d").to_string(),
                        start_time: start_time.format("%H:%M").to_string(),
                        end_time: end_time.format("%H:%M").to_string(),
                        clips: current_session_clips,
                    };
                    sessions.push(session);
                }
            }
        }

        sessions.reverse(); // Show newest sessions first
        sessions
    }

    /// Ensures video info is loaded for a specific clip index
    /// Used for background loading when clips are displayed
    fn ensure_video_info_loaded(&mut self, clip_index: usize) {
        if let Some(clip) = self.clips.get_mut(clip_index) {
            if clip.needs_video_info_update() {
                match clip.populate_video_info() {
                    Ok(is_valid) => {
                        if is_valid {
                            log::debug!("Video info updated for {}", clip.get_output_filename());
                        } else {
                            log::debug!("Video info checked but file still being written: {}", clip.get_output_filename());
                        }
                    }
                    Err(e) => {
                        log::debug!("Failed to update video info for {}: {}", clip.get_output_filename(), e);
                    }
                }
            }
        }
    }

    /// Periodically updates video info for clips that need it
    /// This ensures that clips being written by OBS get updated when they're finished
    /// 
    /// IMPORTANT: This method is called every frame from the main update loop to ensure
    /// that grayed-out files (files still being written by OBS) get their video info
    /// updated as soon as the file becomes valid. This provides a smooth user experience
    /// where files automatically transition from grayed-out to selectable.
    fn update_pending_video_info(&mut self) {
        let now = std::time::Instant::now();
        
        // Check every 2 seconds to avoid excessive file system operations
        if now.duration_since(self.last_video_info_check).as_secs() >= 2 {
            self.last_video_info_check = now;
            
            let mut updated_count = 0;
            for clip in &mut self.clips {
                if clip.needs_video_info_update() {
                    match clip.populate_video_info() {
                        Ok(is_valid) => {
                            if is_valid {
                                log::debug!("Video info updated for {}", clip.get_output_filename());
                                updated_count += 1;
                            }
                        }
                        Err(_) => {
                            // File might not exist yet or still being written, ignore error
                        }
                    }
                }
            }
            
            if updated_count > 0 {
                log::info!("Updated video info for {} clips", updated_count);
            }
        }
    }
    
    fn process_pending_clip_retries(&mut self) {
        let now = std::time::Instant::now();
        let mut requests_to_remove = Vec::new();
        let mut clips_to_update = Vec::new();
        let mut files_to_create = Vec::new();
        
        for (i, request) in self.pending_clip_requests.iter_mut().enumerate() {
            // Check if it's time to retry (every 1 second)
            if now.duration_since(request.last_retry).as_secs() >= 1 {
                request.last_retry = now;
                request.retry_count += 1;
                
                // Check if we've exceeded 10 seconds (10 retries)
                if now.duration_since(request.created_at).as_secs() >= 10 {
                    requests_to_remove.push(i);
                    continue;
                }
                
                // Try to find a matching clip again
                let mut found_existing = false;
                for (clip_index, clip) in self.clips.iter().enumerate() {
                    if clip.matches_timestamp(request.timestamp) {
                        clips_to_update.push((clip_index, request.duration.clone()));
                        found_existing = true;
                        requests_to_remove.push(i);
                        break;
                    }
                }
                
                // If still no match, check for new files
                if !found_existing {
                    if let Some(ref watched_dir) = self.watched_directory {
                        if let Ok(existing_files) = FileMonitor::scan_existing_files(watched_dir) {
                            for file in existing_files {
                                if Self::timestamps_match_static(request.timestamp, file.timestamp) {
                                    files_to_create.push((file, request.duration.clone()));
                                    requests_to_remove.push(i);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Apply updates outside the iteration to avoid borrow conflicts
        for (clip_index, duration) in clips_to_update {
            if let Some(clip) = self.clips.get_mut(clip_index) {
                clip.set_target_duration(duration);
            }
        }
        
        for (file, duration) in files_to_create {
            self.create_clip_from_file(file, Some(duration));
        }
        
        // Remove completed or expired requests (in reverse order to maintain indices)
        for &index in requests_to_remove.iter().rev() {
            self.pending_clip_requests.remove(index);
        }
    }
    
    fn perform_initial_scan(&mut self) {
        if !self.initial_scan_completed {
            if let Some(ref dir) = self.watched_directory.clone() {
                log::info!("Performing initial file scan of {}", dir.display());
                match FileMonitor::scan_existing_files(dir) {
                    Ok(existing_files) => {
                        log::info!("Found {} existing replay files, loading most recent 50", existing_files.len());
                        for file in existing_files.into_iter().take(50) {
                            match Clip::new_without_target(file.path) {
                                Ok(clip) => {
                                    self.clips.push(clip);
                                }
                                Err(e) => {
                                    log::error!("Failed to create clip from existing file: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to scan existing files: {}", e);
                    }
                }
            }
            self.initial_scan_completed = true;
        }
    }


}

impl eframe::App for ClipHelperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Perform initial file scan if not done yet (non-blocking after UI is shown)
        self.perform_initial_scan();
        
        // Process events
        self.process_hotkey_events();
        self.process_file_events();
        
        // Update video info for clips that might still be writing
        self.update_pending_video_info();
        
        // Check for pending clip request retries
        self.process_pending_clip_retries();
        
        // Periodic cleanup of old clip requests
        let cutoff = chrono::Local::now() - chrono::Duration::seconds(30);
        self.pending_clip_requests.retain(|req| req.timestamp > cutoff);
        
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

        egui::SidePanel::left("clip_list")
            .default_width(300.0)
            .min_width(250.0)
            .show(ctx, |ui| {
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
        
        // Show clips grouped by sessions
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.clips.is_empty() {
                    ui.label("No clips loaded");
                    ui.small("Press the scan button above to load existing replay files");
                    ui.small("Or trigger a hotkey to capture new clips");
                } else {
                    let sessions = self.group_clips_into_sessions();
                    let mut selected_index = self.selected_clip_index;
                    let mut clips_needing_info = Vec::new();
                    
                    for session in sessions {
                        // Session header
                        ui.group(|ui| {
                            ui.label(format!("{} - session {} - {}", 
                                session.date, session.start_time, session.end_time));
                        });
                        
                        ui.indent("session_clips", |ui| {
                            for &clip_index in &session.clips {
                                if let Some(clip) = self.clips.get(clip_index) {
                                    let is_selected = selected_index == Some(clip_index);
                                    let is_valid = clip.is_video_valid();
                                    
                                    // Make the entire container clickable and take full width
                                    let container_rect = egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        egui::Vec2::new(ui.available_width(), 10.0 + ui.text_style_height(&egui::TextStyle::Body) * 3.0)
                                    );
                                    
                                    // Draw the container background FIRST (before content)
                                    if is_selected {
                                        ui.painter().rect_filled(container_rect, 4.0, ui.visuals().selection.bg_fill);
                                    } else if container_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                                        // Use a more subtle hover background - lighter version of selection color
                                        let mut hover_color = ui.visuals().selection.bg_fill;
                                        hover_color[3] = (hover_color[3] as f32 * 0.3) as u8; // Make it 30% opacity
                                        ui.painter().rect_filled(container_rect, 4.0, hover_color);
                                    }
                                    
                                    // Draw border only for selected items
                                    if is_selected {
                                        ui.painter().rect_stroke(container_rect, 4.0, ui.visuals().selection.stroke);
                                    }
                                    
                                    // Content area inside the container with 5px padding
                                    let content_rect = container_rect.shrink(5.0);
                                    
                                    // Create the content inside the container
                                    ui.allocate_ui_at_rect(content_rect, |ui| {
                                        ui.scope(|ui| {
                                            // Override text color for invalid files only
                                            if !is_valid {
                                                ui.visuals_mut().override_text_color = Some(egui::Color32::GRAY);
                                            }
                                            
                                            // Filename
                                            ui.label(&clip.get_output_filename());
                                            
                                            // Show video length and target duration on separate lines
                                            if let Some(video_length) = clip.video_length_seconds {
                                                if video_length >= 1.0 {
                                                    ui.small(format!("Video: {}", Clip::format_duration(video_length)));
                                                } else {
                                                    ui.small("Video: Invalid (<1s)");
                                                }
                                            } else {
                                                ui.small("Video: Loading...");
                                                // Mark for background loading
                                                clips_needing_info.push(clip_index);
                                            }
                                            
                                            // Show target duration only if set
                                            if clip.has_target_duration() {
                                                ui.small(format!("Target: {}", Clip::format_duration(clip.duration_seconds as f64)));
                                            } else {
                                                ui.small("Target: Not set");
                                            }
                                        });
                                    });
                                    
                                    // Create an interactive layer over the entire container for clicks
                                    let container_response = ui.interact(container_rect, egui::Id::new(format!("clip_container_{}", clip_index)), egui::Sense::click());
                                    
                                    if container_response.clicked() && is_valid {
                                        selected_index = Some(clip_index);
                                    }
                                    
                                    // Advance cursor past the container (no double allocation)
                                    ui.advance_cursor_after_rect(container_rect);
                                    
                                    ui.add_space(4.0);
                                }
                            }
                        });
                        
                        ui.add_space(8.0);
                    }
                    
                    // Update selected clip
                    if selected_index != self.selected_clip_index {
                        if let Some(index) = selected_index {
                            self.select_clip(index);
                        }
                    }
                    
                    // Load video info for clips that need it (after UI to avoid borrowing issues)
                    for clip_index in clips_needing_info {
                        self.ensure_video_info_loaded(clip_index);
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
                        // Create clips without target duration for existing files
                        let file_path = file.path.clone();
                        match Clip::new_without_target(file.path) {
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
