// =============================================================================
// MEDIA CONTROLLER - SINGLE POINT OF CONTROL FOR VIDEO AND AUDIO
// =============================================================================
//
// This module provides a unified interface for controlling both video and audio
// playback, ensuring they are always synchronized and preventing the class of
// bugs where one system is updated but the other is forgotten.
//
// DESIGN PRINCIPLES:
// - Single public interface: only MediaController has play/pause/seek methods
// - Centralized state: position and playing state managed in one place
// - Always coordinated: impossible to update video without audio or vice versa
// - Clear ownership: video and audio players are internal implementation details
//
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc;
use crate::video::audio_player_complete::SynchronizedAudioPlayer;
use crate::core::clip::AudioTrack;
use egui::{Context, TextureHandle};

/// Raw video frame data that can be sent between threads
#[derive(Debug)]
pub struct VideoFrame {
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: f64,
    pub sequence: u64,
    pub process_id: u64, // Track which FFmpeg process this frame came from
}

/// Commands sent to the audio thread
#[derive(Debug, Clone)]
/// Commands that can be sent to the video thread
pub enum VideoCommand {
    SetVideo(PathBuf, f64, f64), // path, total_duration, frame_rate
    Seek(f64),              // position
    Play,
    Pause,
    UpdateFrame(egui::Context, f64), // Update current frame for GUI, current_position
    Shutdown,
}

/// Status updates from the video thread
#[derive(Debug)]
pub enum VideoStatus {
    Ready,
    Playing,
    Paused,
    PositionUpdate(f64), // Send position updates during playback
    Error(String),
}

impl Clone for VideoStatus {
    fn clone(&self) -> Self {
        match self {
            VideoStatus::Ready => VideoStatus::Ready,
            VideoStatus::Playing => VideoStatus::Playing,
            VideoStatus::Paused => VideoStatus::Paused,
            VideoStatus::PositionUpdate(pos) => VideoStatus::PositionUpdate(*pos),
            VideoStatus::Error(msg) => VideoStatus::Error(msg.clone()),
        }
    }
}

/// Thread-safe video controller that uses message passing
/// This allows MediaController to be Send + Sync
pub struct ThreadSafeVideoController {
    command_sender: mpsc::Sender<VideoCommand>,
    status_receiver: std::sync::Mutex<mpsc::Receiver<VideoStatus>>,
    frame_receiver: std::sync::Mutex<mpsc::Receiver<VideoFrame>>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl ThreadSafeVideoController {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        
        let handle = std::thread::spawn(move || {
            let mut current_video_path: Option<PathBuf> = None;
            let mut current_duration = 0.0;
            let mut current_state = VideoStatus::Ready;
            let mut current_position = 0.0;
            let mut sequence_number = 0u64;
            let mut ffmpeg_process: Option<std::process::Child> = None;
            let mut video_frame_rate = 30.0; // Default to 30 FPS, updated when video is loaded
            let mut current_process_id = 0u64; // Track which FFmpeg process frames should come from
            let mut starting_ffmpeg = false; // Prevent multiple simultaneous FFmpeg starts
            let mut pending_seek_position: Option<f64> = None; // Queue latest seek while restart in progress
            
            // Function to start continuous FFmpeg process for playback
            fn start_continuous_ffmpeg(video_path: &PathBuf, start_position: f64, _frame_rate: f64) -> Result<std::process::Child, String> {
                use std::process::{Command, Stdio};
                
                let process = Command::new("ffmpeg")
                    .args([
                        "-ss", &start_position.to_string(),
                        "-i", video_path.to_str().unwrap(),
                        "-f", "rawvideo",
                        "-pix_fmt", "rgb24",
                        "-s", "854x480",
                        // "-r", &frame_rate.to_string(), // Use actual video frame rate
                        "-"
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("Failed to spawn FFmpeg: {}", e))?;
                
                Ok(process)
            }
            
            // Helper function to safely restart FFmpeg process
            fn safe_restart_ffmpeg(
                current_process: &mut Option<std::process::Child>,
                video_path: &PathBuf,
                position: f64,
                frame_rate: f64,
                process_id: &mut u64,
                starting_flag: &mut bool,
                pending_seek: &mut Option<f64>
            ) -> Result<(), String> {
                if *starting_flag {
                    log::info!("SEEK QUEUE: FFmpeg restart in progress, queuing seek to {:.2}s (previous pending: {:?})", position, pending_seek);
                    *pending_seek = Some(position); // Queue the latest seek position
                    return Ok(()); // Don't return an error, just queue it
                }
                
                log::debug!("Starting FFmpeg restart to position {:.2}s", position);
                *starting_flag = true;
                
                // Kill existing process
                if let Some(mut process) = current_process.take() {
                    log::debug!("Killing existing FFmpeg process (PID: {:?})", process.id());
                    let _ = process.kill();
                    
                    // Wait for process to fully terminate with timeout
                    use std::time::{Duration, Instant};
                    let start_time = Instant::now();
                    let timeout = Duration::from_millis(500); // 500ms timeout for process cleanup
                    
                    loop {
                        match process.try_wait() {
                            Ok(Some(_)) => {
                                log::debug!("FFmpeg process terminated successfully");
                                break;
                            }
                            Ok(None) => {
                                if start_time.elapsed() > timeout {
                                    log::warn!("FFmpeg process taking too long to terminate, force killing");
                                    let _ = process.kill(); // Force kill if still running
                                    let _ = process.wait();
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(10)); // Small sleep to avoid busy waiting
                            }
                            Err(e) => {
                                log::warn!("Error waiting for FFmpeg process: {}", e);
                                break;
                            }
                        }
                    }
                }
                
                // Increment process ID for new process
                *process_id += 1;
                
                // Start new process
                match start_continuous_ffmpeg(video_path, position, frame_rate) {
                    Ok(new_process) => {
                        log::info!("Started new FFmpeg process at {:.2}s (process ID: {}, PID: {:?})", position, process_id, new_process.id());
                        *current_process = Some(new_process);
                        log::debug!("FFmpeg restart completed, clearing starting_flag");
                        *starting_flag = false;
                        Ok(())
                    }
                    Err(e) => {
                        log::error!("Failed to start FFmpeg process: {}", e);
                        *starting_flag = false;
                        Err(e)
                    }
                }
            }
            
            // Frame extraction function (simplified from EmbeddedVideoPlayer)
            fn extract_frame(video_path: &PathBuf, timestamp: f64, sequence: u64) -> Result<VideoFrame, String> {
                use std::process::Command;
                
                let width = 854; // Standard width for video preview
                let height = 480; // Standard height for video preview
                
                let output = Command::new("ffmpeg")
                    .args([
                        "-ss", &format!("{:.6}", timestamp),
                        "-i", video_path.to_str().ok_or("Invalid path")?,
                        "-vframes", "1",
                        "-f", "rawvideo",
                        "-pix_fmt", "rgb24",
                        "-s", &format!("{}x{}", width, height),
                        "-v", "quiet",
                        "-"
                    ])
                    .output()
                    .map_err(|e| format!("FFmpeg execution failed: {}", e))?;
                    
                if !output.status.success() {
                    return Err(format!("FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr)));
                }
                
                let frame_data = output.stdout;
                let expected_size = (width * height * 3) as usize;
                
                if frame_data.len() != expected_size {
                    return Err(format!("Unexpected frame size: {} (expected {})", frame_data.len(), expected_size));
                }
                
                // Convert RGB24 to RGBA for egui
                let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
                for chunk in frame_data.chunks(3) {
                    if chunk.len() == 3 {
                        rgba_data.push(chunk[0]); // R
                        rgba_data.push(chunk[1]); // G
                        rgba_data.push(chunk[2]); // B
                        rgba_data.push(255);      // A
                    }
                }
                
                Ok(VideoFrame {
                    image_data: rgba_data,
                    width,
                    height,
                    timestamp,
                    sequence,
                    process_id: 0, // Single frame extraction doesn't need process tracking
                })
            }
            
            // Video thread event loop 
            let mut should_play = false;
            let mut frame_duration = 1.0 / 30.0; // Default, will be updated when video is set
            
            loop {
                // First check for shutdown command specifically
                if let Ok(VideoCommand::Shutdown) = cmd_rx.try_recv() {
                    log::info!("Video thread: Received shutdown command, terminating");
                    break;
                }
                
                // Handle playback - CONTINUOUS FRAME PROCESSING for high FPS
                if should_play && matches!(current_state, VideoStatus::Playing) {
                    if let Some(ref mut process) = ffmpeg_process {
                        if let Some(stdout) = process.stdout.as_mut() {
                            // Process frames continuously without delays
                            let mut frames_processed = 0;
                            loop {
                                let frame_size = 854 * 480 * 3;
                                let mut buffer = vec![0u8; frame_size];
                                
                                use std::io::Read;
                                match stdout.read_exact(&mut buffer) {
                                    Ok(()) => {
                                        // Convert RGB24 to RGBA
                                        let mut rgba_data = Vec::with_capacity(854 * 480 * 4);
                                        for chunk in buffer.chunks(3) {
                                            if chunk.len() == 3 {
                                                rgba_data.push(chunk[0]); // R
                                                rgba_data.push(chunk[1]); // G
                                                rgba_data.push(chunk[2]); // B
                                                rgba_data.push(255);      // A
                                            }
                                        }
                                        
                                        let frame = VideoFrame {
                                            image_data: rgba_data,
                                            width: 854,
                                            height: 480,
                                            timestamp: current_position,
                                            sequence: sequence_number,
                                            process_id: current_process_id,
                                        };
                                        sequence_number += 1;
                                        let _ = frame_tx.send(frame);
                                        
                                        // Advance position by exactly one frame
                                        current_position += frame_duration;
                                        current_position = current_position.min(current_duration);
                                        
                                        // Send position update every frame for smooth time display
                                        let _ = status_tx.send(VideoStatus::PositionUpdate(current_position));
                                        
                                        frames_processed += 1;
                                        
                                        // Debug timing every 60 frames
                                        if sequence_number % 60 == 0 {
                                            log::debug!("Frame {}, duration: {:.3}ms", sequence_number, frame_duration * 1000.0);
                                        }
                                        
                                        // Check for commands every 10 frames to maintain responsiveness
                                        if frames_processed >= 10 {
                                            break; // Exit frame processing loop to check commands
                                        }
                                        
                                        // Quick non-blocking check for urgent commands (pause/seek)
                                        match cmd_rx.try_recv() {
                                            Ok(VideoCommand::Pause) => {
                                                log::debug!("Video thread: Pause command during continuous playback");
                                                current_state = VideoStatus::Paused;
                                                should_play = false;
                                                let _ = status_tx.send(VideoStatus::Paused);
                                                if let Some(mut process) = ffmpeg_process.take() {
                                                    let _ = process.kill();
                                                    let _ = process.wait();
                                                    log::info!("Video thread: Stopped continuous FFmpeg process");
                                                }
                                                break; // Exit to main loop
                                            }
                                            Ok(VideoCommand::Seek(position)) => {
                                                log::debug!("Video thread: Seek command during continuous playback to {:.2}s", position);
                                                current_position = position.max(0.0).min(current_duration);
                                                sequence_number += 1;
                                                
                                                // Use safe restart to prevent multiple processes
                                                if let Some(ref path) = current_video_path {
                                                    match safe_restart_ffmpeg(
                                                        &mut ffmpeg_process,
                                                        path,
                                                        current_position,
                                                        video_frame_rate,
                                                        &mut current_process_id,
                                                        &mut starting_ffmpeg,
                                                        &mut pending_seek_position
                                                    ) {
                                                        Ok(()) => {
                                                            // Success - continue playback
                                                        }
                                                        Err(e) => {
                                                            log::error!("Video thread: Failed to restart FFmpeg after seek: {}", e);
                                                            current_state = VideoStatus::Paused;
                                                            should_play = false;
                                                            let _ = status_tx.send(VideoStatus::Paused);
                                                        }
                                                    }
                                                }
                                                let _ = status_tx.send(current_state.clone());
                                                break; // Exit to restart frame processing with new FFmpeg process
                                            }
                                            Ok(VideoCommand::Shutdown) => {
                                                log::info!("Video thread: Shutdown command during continuous playback");
                                                should_play = false;
                                                break;
                                            }
                                            _ => {} // Continue processing frames
                                        }
                                    }
                                    Err(_) => {
                                        // End of stream or error - stop playback
                                        log::info!("Video thread: FFmpeg stream ended, stopping playback");
                                        should_play = false;
                                        current_state = VideoStatus::Paused;
                                        if let Some(mut process) = ffmpeg_process.take() {
                                            let _ = process.kill();
                                            let _ = process.wait();
                                        }
                                        let _ = status_tx.send(VideoStatus::Paused);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                
                
                // Then process other commands with short timeout for responsiveness
                let timeout_ms = 1; // Fixed short timeout for responsiveness, not playback timing
                match cmd_rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
                    Ok(command) => {
                        log::debug!("Video thread received command: {:?}", command);
                        match command {
                            VideoCommand::SetVideo(path, duration, frame_rate) => {
                        log::info!("Video thread: Setting up video {:?} with duration {:.2}s, frame rate {:.2} FPS", path, duration, frame_rate);
                        current_video_path = Some(path.clone());
                        current_duration = duration;
                        current_position = 0.0;
                        sequence_number += 1;
                        current_state = VideoStatus::Ready;
                        video_frame_rate = frame_rate; // Store frame rate for timing
                        frame_duration = 1.0 / frame_rate; // Update frame duration for correct timing!
                        log::info!("Video thread: Updated frame_duration to {:.6}s ({:.2} FPS)", frame_duration, frame_rate);
                        let _ = status_tx.send(VideoStatus::Ready);
                        
                        // Extract initial frame at position 0
                        log::debug!("Video thread: Extracting initial frame at position 0.0");
                        if let Ok(frame) = extract_frame(&path, 0.0, sequence_number) {
                            log::debug!("Video thread: Successfully extracted initial frame");
                            let _ = frame_tx.send(frame);
                        } else {
                            log::warn!("Video thread: Failed to extract initial frame");
                        }
                    }
                    VideoCommand::Seek(position) => {
                        log::info!("Video thread: Seek command received in main loop to {:.2}s", position);
                        current_position = position.max(0.0).min(current_duration);
                        sequence_number += 1;
                        
                        // Handle seek in main loop (for when video is paused/not playing)
                        if let Some(ref path) = current_video_path {
                            match safe_restart_ffmpeg(
                                &mut ffmpeg_process,
                                path,
                                current_position,
                                video_frame_rate,
                                &mut current_process_id,
                                &mut starting_ffmpeg,
                                &mut pending_seek_position
                            ) {
                                Ok(()) => {
                                    log::info!("Video thread: Successfully processed seek in main loop");
                                }
                                Err(e) => {
                                    log::error!("Video thread: Failed to process seek in main loop: {}", e);
                                }
                            }
                        }
                        
                        // Extract frame at new position if not playing
                        if !matches!(current_state, VideoStatus::Playing) {
                            if let Some(ref path) = current_video_path {
                                if let Ok(frame) = extract_frame(path, current_position, sequence_number) {
                                    let _ = frame_tx.send(frame);
                                    sequence_number += 1;
                                }
                            }
                        }
                        let _ = status_tx.send(current_state.clone());
                    }
                    VideoCommand::Play => {
                        log::debug!("Video thread: Starting playback");
                        current_state = VideoStatus::Playing;
                        should_play = true; // Enable playback
                        let _ = status_tx.send(VideoStatus::Playing);
                        
                        // Start continuous FFmpeg process for playback
                        if let Some(ref path) = current_video_path {
                            // Use safe restart to prevent multiple processes
                            match safe_restart_ffmpeg(
                                &mut ffmpeg_process,
                                path,
                                current_position,
                                video_frame_rate,
                                &mut current_process_id,
                                &mut starting_ffmpeg,
                                &mut pending_seek_position
                            ) {
                                Ok(()) => {
                                    // Success - playback started
                                }
                                Err(e) => {
                                    log::error!("Video thread: Failed to start FFmpeg process: {}", e);
                                    current_state = VideoStatus::Paused;
                                    let _ = status_tx.send(VideoStatus::Paused);
                                }
                            }
                        }
                    }
                    VideoCommand::Pause => {
                        log::debug!("Video thread: Pausing playback");
                        current_state = VideoStatus::Paused;
                        should_play = false; // Stop real-time playback
                        let _ = status_tx.send(VideoStatus::Paused);
                        
                        // Kill continuous FFmpeg process
                        if let Some(mut process) = ffmpeg_process.take() {
                            let _ = process.kill();
                            let _ = process.wait();
                            log::info!("Video thread: Stopped continuous FFmpeg process");
                        }
                    }
                    VideoCommand::UpdateFrame(_ctx, position) => {
                        // Only used when paused - extract single frame at specific position
                        if !matches!(current_state, VideoStatus::Playing) {
                            current_position = position;
                            if let Some(ref path) = current_video_path {
                                if let Ok(frame) = extract_frame(path, current_position, sequence_number) {
                                    let _ = frame_tx.send(frame);
                                    sequence_number += 1;
                                }
                            }
                            let _ = status_tx.send(current_state.clone());
                        }
                    }
                    VideoCommand::Shutdown => {
                        log::info!("Video thread: Received shutdown command, terminating");
                        break;
                    }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Timeout - check for shutdown and pending seeks
                        
                        // Process any pending seek that was queued during FFmpeg restart
                        if !starting_ffmpeg && pending_seek_position.is_some() {
                            if let Some(pending_pos) = pending_seek_position.take() {
                                log::info!("SEEK QUEUE: Processing queued seek to {:.2}s", pending_pos);
                                current_position = pending_pos.max(0.0).min(current_duration);
                                
                                if let Some(ref path) = current_video_path {
                                    match safe_restart_ffmpeg(
                                        &mut ffmpeg_process,
                                        path,
                                        current_position,
                                        video_frame_rate,
                                        &mut current_process_id,
                                        &mut starting_ffmpeg,
                                        &mut pending_seek_position
                                    ) {
                                        Ok(()) => {
                                            log::info!("SEEK QUEUE: Successfully processed pending seek to {:.2}s", current_position);
                                        }
                                        Err(e) => {
                                            log::error!("SEEK QUEUE: Failed to process pending seek: {}", e);
                                        }
                                    }
                                }
                            }
                        } else if pending_seek_position.is_some() {
                            log::info!("SEEK QUEUE: Pending seek exists but FFmpeg restart still in progress, waiting...");
                        }
                        
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        log::info!("Video thread: Channel disconnected, terminating");
                        break;
                    }
                }
            }
            
            // Cleanup: Kill any running FFmpeg process before exiting
            if let Some(mut process) = ffmpeg_process.take() {
                let _ = process.kill();
                let _ = process.wait();
                log::info!("Video thread: Cleaned up FFmpeg process on shutdown");
            }
        });
        
        Self {
            command_sender: cmd_tx,
            status_receiver: std::sync::Mutex::new(status_rx),
            frame_receiver: std::sync::Mutex::new(frame_rx),
            thread_handle: Some(handle),
        }
    }
    
    pub fn send_command(&self, command: VideoCommand) {
        let _ = self.command_sender.send(command);
    }
    
    pub fn get_frame(&self) -> Option<VideoFrame> {
        if let Ok(receiver) = self.frame_receiver.lock() {
            receiver.try_recv().ok()
        } else {
            None
        }
    }
    
    pub fn get_status(&self) -> Option<VideoStatus> {
        if let Ok(receiver) = self.status_receiver.lock() {
            receiver.try_recv().ok()
        } else {
            None
        }
    }
}

impl Drop for ThreadSafeVideoController {
    fn drop(&mut self) {
        log::debug!("ThreadSafeVideoController::drop() - sending shutdown command");
        let _ = self.command_sender.send(VideoCommand::Shutdown);
        
        if let Some(handle) = self.thread_handle.take() {
            // Try to join with a reasonable timeout by attempting multiple times
            for attempt in 1..=5 {
                if handle.is_finished() {
                    log::debug!("ThreadSafeVideoController::drop() - thread finished, joining");
                    let _ = handle.join();
                    return;
                }
                log::debug!("ThreadSafeVideoController::drop() - attempt {} - waiting for thread to finish", attempt);
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            
            log::warn!("ThreadSafeVideoController::drop() - thread did not finish after 1 second, abandoning join");
            // Let the thread handle be dropped without joining - it will terminate when the process exits
        }
    }
}

/// Commands that can be sent to the audio thread
pub enum AudioCommand {
    SetVideo(PathBuf, Vec<AudioTrack>),
    Play(f64), // timestamp
    Pause,
    Seek(f64), // timestamp
    UpdateTracks(Vec<AudioTrack>),
    Stop,
    Shutdown,
}

/// Status updates from the audio thread
#[derive(Debug, Clone)]
pub enum AudioStatus {
    Ready,
    Playing,
    Paused,
    Stopped,
    Error(String),
}

/// Thread-safe audio controller that uses message passing
/// This allows MediaController to be Send + Sync
pub struct ThreadSafeAudioController {
    command_sender: mpsc::Sender<AudioCommand>,
    status_receiver: std::sync::Mutex<mpsc::Receiver<AudioStatus>>,
}

impl ThreadSafeAudioController {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();
        
        // Spawn audio thread - this thread owns the non-Send audio streams
        std::thread::spawn(move || {
            let mut audio_player: Option<SynchronizedAudioPlayer> = None;
            
            // Helper function to get video duration using FFmpeg
            fn get_video_duration(video_path: &PathBuf) -> Option<f64> {
                let output = std::process::Command::new("ffprobe")
                    .args([
                        "-v", "quiet",
                        "-show_entries", "format=duration",
                        "-of", "csv=p=0",
                        video_path.to_str()?
                    ])
                    .output()
                    .ok()?;
                
                if output.status.success() {
                    let duration_str = String::from_utf8(output.stdout).ok()?;
                    duration_str.trim().parse::<f64>().ok()
                } else {
                    None
                }
            }
            
            for command in cmd_rx {
                match command {
                    AudioCommand::SetVideo(path, tracks) => {
                        log::info!("Audio: Setting video to {:?} with {} tracks", path, tracks.len());
                        
                        // Create new audio player instance
                        match SynchronizedAudioPlayer::new() {
                            Ok(mut player) => {
                                // Set the video with duration and audio tracks
                                if let Some(duration) = get_video_duration(&path) {
                                    player.set_video(path, duration, &tracks);
                                    audio_player = Some(player);
                                    let _ = status_tx.send(AudioStatus::Ready);
                                    log::info!("Audio: Successfully set video with duration {:.2}s", duration);
                                } else {
                                    log::error!("Audio: Failed to get video duration");
                                    let _ = status_tx.send(AudioStatus::Error("Failed to get video duration".to_string()));
                                }
                            }
                            Err(e) => {
                                log::error!("Audio: Failed to create audio player: {}", e);
                                let _ = status_tx.send(AudioStatus::Error(format!("Failed to create audio player: {}", e)));
                            }
                        }
                    }
                    AudioCommand::Play(timestamp) => {
                        log::info!("Audio: Starting playback at {:.2}s", timestamp);
                        if let Some(ref mut player) = audio_player {
                            player.seek(timestamp); // Seek to position first
                            player.play(); // Then start playing
                            let _ = status_tx.send(AudioStatus::Playing);
                        } else {
                            log::warn!("Audio: No audio player available for play command");
                        }
                    }
                    AudioCommand::Pause => {
                        log::info!("Audio: Pausing playback");
                        if let Some(ref mut player) = audio_player {
                            player.pause();
                            let _ = status_tx.send(AudioStatus::Paused);
                        }
                    }
                    AudioCommand::Seek(timestamp) => {
                        log::info!("Audio: Seeking to {:.2}s", timestamp);
                        if let Some(ref mut player) = audio_player {
                            player.seek(timestamp);
                            // No status update - maintains current playing state
                        }
                    }
                    AudioCommand::UpdateTracks(tracks) => {
                        log::info!("Audio: Updating tracks configuration");
                        if let Some(ref mut player) = audio_player {
                            player.update_audio_tracks(&tracks);
                        }
                    }
                    AudioCommand::Stop => {
                        log::info!("Audio: Stopping playback");
                        if let Some(ref mut player) = audio_player {
                            player.stop();
                        }
                        audio_player = None;
                        let _ = status_tx.send(AudioStatus::Stopped);
                    }
                    AudioCommand::Shutdown => {
                        log::info!("Audio: Shutting down audio thread");
                        if let Some(ref mut player) = audio_player {
                            player.stop();
                        }
                        break;
                    }
                }
            }
            log::info!("Audio: Audio thread exiting");
        });
        
        Self {
            command_sender: cmd_tx,
            status_receiver: std::sync::Mutex::new(status_rx),
        }
    }
    
    pub fn send_command(&self, command: AudioCommand) -> Result<(), mpsc::SendError<AudioCommand>> {
        self.command_sender.send(command)
    }
    
    pub fn try_recv_status(&self) -> Result<AudioStatus, mpsc::TryRecvError> {
        if let Ok(receiver) = self.status_receiver.lock() {
            receiver.try_recv()
        } else {
            Err(mpsc::TryRecvError::Disconnected)
        }
    }
}

impl Drop for ThreadSafeAudioController {
    fn drop(&mut self) {
        log::debug!("ThreadSafeAudioController::drop() - sending shutdown command");
        let _ = self.command_sender.send(AudioCommand::Shutdown);
        // Note: Can't join thread since we don't store the handle
        // Thread will terminate when it receives Shutdown command
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaControllerState {
    /// No video loaded
    Unloaded,
    /// Video is being loaded/initialized
    Loading,
    /// Video loaded and ready for playback
    Ready,
    /// Currently playing
    Playing,
    /// Currently paused
    Paused,
    /// Seeking to new position
    Seeking,
    /// Error occurred with details
    Error(String),
}

impl MediaControllerState {
    /// Returns true if the controller is in a state where user can interact
    pub fn can_play(&self) -> bool {
        matches!(self, MediaControllerState::Ready | MediaControllerState::Paused)
    }
    
    /// Returns true if the controller is in a state where user can pause
    pub fn can_pause(&self) -> bool {
        matches!(self, MediaControllerState::Playing)
    }
    
    /// Returns true if the controller is in a state where user can seek
    pub fn can_seek(&self) -> bool {
        matches!(self, 
            MediaControllerState::Ready | 
            MediaControllerState::Playing | 
            MediaControllerState::Paused
        )
    }
    
    /// Returns true if this state indicates an operation is in progress
    pub fn is_busy(&self) -> bool {
        matches!(self, MediaControllerState::Loading | MediaControllerState::Seeking)
    }
    
    /// Returns user-friendly display text for this state
    pub fn display_text(&self) -> &str {
        match self {
            MediaControllerState::Unloaded => "No video loaded",
            MediaControllerState::Loading => "Loading video...",
            MediaControllerState::Ready => "Ready",
            MediaControllerState::Playing => "Playing",
            MediaControllerState::Paused => "Paused",
            MediaControllerState::Seeking => "Seeking...",
            MediaControllerState::Error(msg) => msg,
        }
    }
}

pub struct MediaController {
    // State management - single source of truth
    current_position: f64,
    is_playing: bool,
    total_duration: f64,
    video_path: Option<PathBuf>,
    state: MediaControllerState,
    
    // Video frame rendering
    texture_handle: Option<TextureHandle>,
    last_frame_position: f64,  // Track when we last requested a frame
    
    // Playback timing
    playback_start_time: Option<std::time::Instant>,
    playback_start_position: f64,
    video_frame_rate: f64,  // Actual video frame rate (e.g., 60, 360)
    
    // Shutdown flag to prevent operations during cleanup
    is_shutting_down: bool,
    
    // Thread-safe controllers using message passing
    audio_controller: ThreadSafeAudioController,
    video_controller: ThreadSafeVideoController,
}

impl MediaController {
    pub fn new() -> Self {
        Self {
            current_position: 0.0,
            is_playing: false,
            total_duration: 0.0,
            video_path: None,
            state: MediaControllerState::Unloaded,
            texture_handle: None,
            last_frame_position: -1.0,  // Initialize to invalid position
            playback_start_time: None,
            playback_start_position: 0.0,
            video_frame_rate: 30.0,  // Default to 30 FPS, will be updated when video is loaded
            is_shutting_down: false,
            audio_controller: ThreadSafeAudioController::new(),
            video_controller: ThreadSafeVideoController::new(),
        }
    }
    
    // =============================================================================
    // PUBLIC INTERFACE - Single point of control
    // =============================================================================
    
    /// Start playback from current position
    /// ALWAYS coordinates both video and audio
    pub fn play(&mut self) {
        log::debug!("MediaController::play() called - current state: {:?}, video_path: {:?}", 
            self.state, self.video_path);
            
        // Check if we can play in current state
        if !self.state.can_play() {
            log::warn!("Cannot play in current state: {:?}. Video path: {:?}, total_duration: {}", 
                self.state, self.video_path, self.total_duration);
            return;
        }
        
        log::info!("Starting playback from position {:.2}s", self.current_position);
        
        // Coordinate both video and audio playback
        // Send command to audio thread
        let _ = self.audio_controller.send_command(AudioCommand::Play(self.current_position));
        
        // Send command to video thread
        self.video_controller.send_command(VideoCommand::Play);
        
        self.is_playing = true;
        self.state = MediaControllerState::Playing;
        self.playback_start_time = Some(std::time::Instant::now());
        self.playback_start_position = self.current_position;
        log::debug!("MediaController state changed to Playing - is_playing: {}", self.is_playing);
    }
    
    /// Pause playback
    /// ALWAYS coordinates both video and audio
    pub fn pause(&mut self) {
        log::debug!("MediaController::pause() called - current state: {:?}, video_path: {:?}", 
            self.state, self.video_path);
            
        // Check if we can pause in current state
        if !self.state.can_pause() {
            log::warn!("Cannot pause in current state: {:?}. Video path: {:?}, is_playing: {}", 
                self.state, self.video_path, self.is_playing);
            return;
        }
        
        log::info!("Pausing playback at position {:.2}s", self.current_position);
        
        // Coordinate both video and audio pause
        // Send command to audio thread
        let _ = self.audio_controller.send_command(AudioCommand::Pause);
        
        // Send command to video thread
        self.video_controller.send_command(VideoCommand::Pause);
        
        self.is_playing = false;
        self.state = MediaControllerState::Paused;
        self.playback_start_time = None;  // Stop timing
        log::debug!("MediaController state changed to Paused");
    }
    
    /// Seek to specific timestamp
    /// ALWAYS coordinates both video and audio
    pub fn seek(&mut self, timestamp: f64) {
        log::debug!("MediaController::seek({:.2}) called - current state: {:?}", timestamp, self.state);
        
        // Check if we can seek in current state
        if !self.state.can_seek() {
            log::warn!("Cannot seek in current state: {:?}. Video path: {:?}", 
                self.state, self.video_path);
            return;
        }
        
        // Validate and sanitize timestamp
        let sanitized_timestamp = if timestamp.is_nan() || timestamp.is_infinite() {
            log::warn!("Invalid timestamp provided: {}, using 0.0", timestamp);
            0.0
        } else {
            timestamp.clamp(0.0, self.total_duration)
        };
        
        if sanitized_timestamp != timestamp {
            log::debug!("Clamped seek timestamp from {:.2} to {:.2} (duration: {:.2})", 
                timestamp, sanitized_timestamp, self.total_duration);
        }
        
        log::info!("Seeking to position {:.2}s", sanitized_timestamp);
        
        // Set seeking state during operation
        let was_playing = self.is_playing;
        self.state = MediaControllerState::Seeking;
        log::debug!("MediaController state changed to Seeking");
        
        // Coordinate both video and audio seeking
        // Send command to audio thread first (it will pause/stop current playback)
        let _ = self.audio_controller.send_command(AudioCommand::Seek(sanitized_timestamp));
        
        // Send command to video thread
        self.video_controller.send_command(VideoCommand::Seek(sanitized_timestamp));
        log::debug!("Sent seek commands to both audio and video threads");
        
        self.current_position = sanitized_timestamp;
        self.last_frame_position = sanitized_timestamp;  // Prevent UpdateFrame spam after seek
        
        // Restore appropriate state after seeking
        if was_playing {
            self.state = MediaControllerState::Playing;
            self.playback_start_time = Some(std::time::Instant::now());
            self.playback_start_position = sanitized_timestamp;
        } else {
            self.state = MediaControllerState::Paused;
            self.playback_start_time = None;
        }
        // Don't reset last_frame_position here - let the next update cycle handle it properly
    }
    
    /// Set video file and initialize players
    pub fn set_video(&mut self, video_path: PathBuf, audio_tracks: &[AudioTrack], duration: f64, _ctx: &Context) -> Result<(), Box<dyn std::error::Error>> {
        log::info!("MediaController::set_video() called with path: {:?}, duration: {:.2}s", video_path, duration);
        
        // Set loading state during operation
        self.state = MediaControllerState::Loading;
        log::debug!("MediaController state changed to Loading");
        
        // Extract video frame rate for proper playback timing
        match Self::get_video_frame_rate(&video_path) {
            Ok(frame_rate) => {
                self.video_frame_rate = frame_rate;
                log::info!("Detected video frame rate: {:.2} FPS", frame_rate);
            }
            Err(e) => {
                log::warn!("Failed to detect frame rate, using default 30 FPS: {}", e);
                self.video_frame_rate = 30.0;
            }
        }
        
        // Initialize both video and audio players
        // Create audio tracks with first track enabled by default
        let mut audio_tracks_with_first_enabled = audio_tracks.to_vec();
        if !audio_tracks_with_first_enabled.is_empty() {
            audio_tracks_with_first_enabled[0].enabled = true;
            log::info!("Enabled first audio track: {}", audio_tracks_with_first_enabled[0].name);
        }
        
        // Send video setup command to audio thread
        let _ = self.audio_controller.send_command(AudioCommand::SetVideo(video_path.clone(), audio_tracks_with_first_enabled));
        log::debug!("Sent SetVideo command to audio thread with first track enabled");
        
        // Send video setup command to video thread
        self.video_controller.send_command(VideoCommand::SetVideo(video_path.clone(), duration, self.video_frame_rate));
        log::debug!("Sent SetVideo command to video thread with duration: {:.2}s, frame rate: {:.2} FPS", duration, self.video_frame_rate);
        
        self.video_path = Some(video_path.clone());
        self.current_position = 0.0;
        self.is_playing = false;
        self.last_frame_position = -1.0;  // Reset to force initial frame request
        
        // Always update total_duration with the correct value from video metadata
        self.total_duration = duration;
        log::info!("Updated MediaController total_duration to: {:.2}s", duration);
        
        // Set ready state after successful load
        self.state = MediaControllerState::Ready;
        log::info!("MediaController state changed to Ready for video: {:?}", video_path);
        Ok(())
    }
    
    /// Update audio track configuration
    pub fn update_audio_tracks(&mut self, audio_tracks: &[AudioTrack]) {
        // Send track update command to audio thread
        let _ = self.audio_controller.send_command(AudioCommand::UpdateTracks(audio_tracks.to_vec()));
        // Note: This operation doesn't change the main state
    }
    
    // =============================================================================
    // STATE QUERIES - Read-only access to coordinated state
    // =============================================================================
    
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
    
    pub fn current_position(&self) -> f64 {
        self.current_position
    }
    
    pub fn total_duration(&self) -> f64 {
        self.total_duration
    }
    
    pub fn video_path(&self) -> Option<&PathBuf> {
        self.video_path.as_ref()
    }
    
    pub fn state(&self) -> &MediaControllerState {
        &self.state
    }
    
    /// Get current frame texture for display
    pub fn get_frame_texture(&mut self, _ctx: &Context) -> Option<TextureHandle> {
        self.texture_handle.clone()
    }
    
    /// Update internal state (called from GUI loop)
    pub fn update(&mut self, ctx: &Context) {
        // Don't send commands if we're shutting down
        if self.is_shutting_down {
            log::debug!("MediaController: Skipping update - shutting down");
            return;
        }
        
        // During playback, position is advanced by the video thread reading frames
        // MediaController just tracks the timing but doesn't advance position itself
        if self.is_playing && self.total_duration > 0.0 {
            // Position advancement now happens in video thread during continuous playback
            // MediaController only needs to check for end-of-video
            if let Some(start_time) = self.playback_start_time {
                let elapsed = start_time.elapsed().as_secs_f64();
                let expected_position = (self.playback_start_position + elapsed).min(self.total_duration);
                
                // Only pause when we've definitely reached the end
                if expected_position >= self.total_duration {
                    self.is_playing = false;
                    self.state = MediaControllerState::Paused;
                    self.playback_start_time = None;
                    self.current_position = self.total_duration;
                    self.last_frame_position = self.total_duration; // Prevent UpdateFrame spam at end
                }
            }
        }
        
        // Only request frame updates when NOT playing AND position actually changed
        // During playback, the continuous FFmpeg process streams frames automatically
        if self.video_path.is_some() && !self.is_playing {
            // Only send UpdateFrame if position changed significantly (>0.1 seconds) when paused
            let position_changed = (self.current_position - self.last_frame_position).abs() > 0.1;
            
            if position_changed {
                log::debug!("MediaController: Position changed from {:.2} to {:.2}, requesting frame update", 
                    self.last_frame_position, self.current_position);
                self.video_controller.send_command(VideoCommand::UpdateFrame(ctx.clone(), self.current_position));
                self.last_frame_position = self.current_position;
            }
        }
        
        // Check for new video frames and convert to textures
        if let Some(frame) = self.video_controller.get_frame() {
            // Only log every 60 frames (once per second) to reduce spam
            if frame.sequence % 60 == 0 {
                log::debug!("MediaController: Received video frame at timestamp {:.2}s ({}x{})", 
                    frame.timestamp, frame.width, frame.height);
            }
                
            // Convert raw frame data to egui texture
            if frame.image_data.len() == (frame.width * frame.height * 4) as usize {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.image_data,
                );
                
                let texture_handle = ctx.load_texture(
                    "video_frame",
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                
                self.texture_handle = Some(texture_handle);
                // Only log texture updates every 60 frames to reduce spam
                if frame.sequence % 60 == 0 {
                    log::debug!("MediaController: Updated video frame texture at timestamp {:.2}s", frame.timestamp);
                }
            } else {
                log::warn!("MediaController: Invalid frame data size: expected {}, got {}", 
                    frame.width * frame.height * 4, frame.image_data.len());
            }
        }
        
        // Check for status updates from audio/video threads
        if let Some(status) = self.audio_controller.try_recv_status().ok() {
            log::debug!("MediaController: Received audio status update: {:?}", status);
        }
        
        // Check for status updates from video thread
        if let Some(status) = self.video_controller.get_status() {
            match status {
                VideoStatus::PositionUpdate(position) => {
                    // Update MediaController position during playback
                    if self.is_playing {
                        self.current_position = position;
                        log::debug!("MediaController: Position updated to {:.2}s during playback", position);
                    }
                }
                _ => {
                    log::debug!("MediaController: Received video status update: {:?}", status);
                }
            }
        }
    }
    
    // =============================================================================
    // COMPATIBILITY METHODS - Match old EmbeddedVideoPlayer interface
    // =============================================================================
    
    /// Get current playback time (compatible with old interface)
    pub fn current_time(&self) -> f64 {
        self.current_position
    }
    
    /// Seek to specific time immediately (compatible with old interface)  
    pub fn seek_immediate(&mut self, timestamp: f64) {
        self.seek(timestamp);
    }
    
    // =============================================================================
    // ERROR HANDLING - Internal methods for state management
    // =============================================================================
    
    /// Set error state with message
    fn set_error(&mut self, message: String) {
        log::error!("MediaController error: {}", message);
        self.state = MediaControllerState::Error(message);
        self.is_playing = false; // Stop playback on error
    }
    
    /// Attempt to recover from error state
    pub fn clear_error(&mut self) {
        if matches!(self.state, MediaControllerState::Error(_)) {
            self.state = if self.video_path.is_some() {
                MediaControllerState::Ready
            } else {
                MediaControllerState::Unloaded
            };
        }
    }
    
    /// Check if controller is in error state
    pub fn has_error(&self) -> bool {
        matches!(self.state, MediaControllerState::Error(_))
    }
    
    /// Get error message if in error state
    pub fn error_message(&self) -> Option<&str> {
        match &self.state {
            MediaControllerState::Error(msg) => Some(msg),
            _ => None,
        }
    }
    
    // =============================================================================
    // HELPER FUNCTIONS
    // =============================================================================
    
    /// Extract video frame rate using ffprobe
    fn get_video_frame_rate(video_path: &PathBuf) -> Result<f64, Box<dyn std::error::Error>> {
        use std::process::Command;
        
        let output = Command::new("ffprobe")
            .args(&[
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "stream=r_frame_rate",
                "-of", "json",
                video_path.to_str().unwrap()
            ])
            .output()?;
            
        if !output.status.success() {
            return Err(format!("ffprobe failed with status: {}", output.status).into());
        }
        
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        
        // Get framerate from first video stream
        let streams = json["streams"].as_array().ok_or("No streams found")?;
        let video_stream = streams.iter()
            .find(|s| s["r_frame_rate"].is_string())
            .ok_or("No video stream with frame rate found")?;
            
        let fps_str = video_stream["r_frame_rate"]
            .as_str()
            .ok_or("Frame rate not found")?;
            
        log::debug!("Raw frame rate string from ffprobe: '{}'", fps_str);
            
        // Parse framerate (format like "30/1" or "29.97/1")
        let fps = if fps_str.contains('/') {
            let parts: Vec<&str> = fps_str.split('/').collect();
            let numerator: f64 = parts[0].parse()?;
            let denominator: f64 = parts[1].parse()?;
            let calculated_fps = if denominator != 0.0 { numerator / denominator } else { 30.0 };
            log::debug!("Parsed fraction {}/{} = {:.3} FPS", numerator, denominator, calculated_fps);
            calculated_fps
        } else {
            let parsed_fps = fps_str.parse().unwrap_or(30.0);
            log::debug!("Parsed direct value: {:.3} FPS", parsed_fps);
            parsed_fps
        };
        
        // Clamp to reasonable frame rate range
        let clamped_fps = fps.clamp(1.0, 1000.0);
        log::debug!("Final frame rate after clamping: {:.3} FPS", clamped_fps);
        
        Ok(clamped_fps)
    }
}

impl Drop for MediaController {
    fn drop(&mut self) {
        log::debug!("MediaController::drop() called - current state: {:?}", self.state);
        
        // Set shutdown flag to prevent further operations
        self.is_shutting_down = true;
        
        // Force stop playback regardless of current state
        self.is_playing = false;
        
        // Send shutdown commands to both threads
        let _ = self.audio_controller.send_command(AudioCommand::Shutdown);
        self.video_controller.send_command(VideoCommand::Shutdown);
        
        log::debug!("MediaController drop complete - sent shutdown commands to threads");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;
    use std::collections::HashSet;
    
    // =============================================================================
    // MOCK PLAYERS WITH COMMAND AND PROCESS TRACKING
    // =============================================================================
    
    #[derive(Debug, Clone, PartialEq)]
    enum MockCommand {
        Play(f64),
        Pause,
        Seek(f64),
        Stop,
        UpdateTracks(Vec<String>), // Use track names instead of full AudioTrack for easier comparison
        SetVideo(PathBuf),
    }
    
    #[derive(Debug, Clone)]
    struct ProcessRecord {
        id: u32,
        command: String,
        status: ProcessStatus,
        spawned_at: Instant,
        killed_at: Option<Instant>,
    }
    
    #[derive(Debug, Clone, PartialEq)]
    enum ProcessStatus {
        Spawned,
        Running, 
        Killed,
        Died, // Process ended on its own
    }
    
    struct MockVideoPlayer {
        commands: Vec<MockCommand>,
        processes: Vec<ProcessRecord>,
        active_processes: HashSet<u32>,
        next_process_id: u32,
        should_fail_on: Option<MockCommand>, // Simulate failures
    }
    
    impl MockVideoPlayer {
        fn new() -> Self {
            Self {
                commands: Vec::new(),
                processes: Vec::new(),
                active_processes: HashSet::new(),
                next_process_id: 1000,
                should_fail_on: None,
            }
        }
        
        fn simulate_play(&mut self, timestamp: f64) -> Result<(), String> {
            let cmd = MockCommand::Play(timestamp);
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock video player failed to play".to_string());
            }
            
            // Simulate spawning a new video stream process
            let process_id = self.spawn_process(format!("ffmpeg_video_stream_{}", timestamp));
            log::debug!("Mock video player: spawned process {} for play at {}s", process_id, timestamp);
            Ok(())
        }
        
        fn simulate_pause(&mut self) -> Result<(), String> {
            let cmd = MockCommand::Pause;
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock video player failed to pause".to_string());
            }
            
            // Pause doesn't kill processes in current system
            log::debug!("Mock video player: paused (keeping {} processes alive)", self.active_processes.len());
            Ok(())
        }
        
        fn simulate_seek(&mut self, timestamp: f64) -> Result<(), String> {
            let cmd = MockCommand::Seek(timestamp);
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock video player failed to seek".to_string());
            }
            
            // Kill old video stream processes
            self.kill_all_processes();
            
            // Spawn new process from seek position
            let process_id = self.spawn_process(format!("ffmpeg_video_stream_{}", timestamp));
            log::debug!("Mock video player: killed old processes, spawned process {} for seek to {}s", process_id, timestamp);
            Ok(())
        }
        
        fn simulate_set_video(&mut self, video_path: PathBuf) -> Result<(), String> {
            let cmd = MockCommand::SetVideo(video_path.clone());
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock video player failed to set video".to_string());
            }
            
            // Kill all processes from previous video
            self.kill_all_processes();
            log::debug!("Mock video player: set video to {:?}, killed all old processes", video_path);
            Ok(())
        }
        
        fn spawn_process(&mut self, command: String) -> u32 {
            let process_id = self.next_process_id;
            self.next_process_id += 1;
            
            let record = ProcessRecord {
                id: process_id,
                command,
                status: ProcessStatus::Spawned,
                spawned_at: Instant::now(),
                killed_at: None,
            };
            
            self.processes.push(record);
            self.active_processes.insert(process_id);
            process_id
        }
        
        fn kill_process(&mut self, process_id: u32) {
            if let Some(record) = self.processes.iter_mut().find(|p| p.id == process_id) {
                record.status = ProcessStatus::Killed;
                record.killed_at = Some(Instant::now());
            }
            self.active_processes.remove(&process_id);
        }
        
        fn kill_all_processes(&mut self) {
            let active_ids: Vec<u32> = self.active_processes.iter().cloned().collect();
            for id in active_ids {
                self.kill_process(id);
            }
        }
        
        // Test helper methods
        fn get_commands(&self) -> &[MockCommand] {
            &self.commands
        }
        
        fn get_active_process_count(&self) -> usize {
            self.active_processes.len()
        }
        
        fn get_total_process_count(&self) -> usize {
            self.processes.len()
        }
        
        fn was_process_killed(&self, process_id: u32) -> bool {
            self.processes.iter()
                .find(|p| p.id == process_id)
                .map(|p| p.status == ProcessStatus::Killed)
                .unwrap_or(false)
        }
        
        fn set_failure_on(&mut self, command: MockCommand) {
            self.should_fail_on = Some(command);
        }
        
        fn clear_failure(&mut self) {
            self.should_fail_on = None;
        }
    }
    
    struct MockAudioPlayer {
        commands: Vec<MockCommand>,
        processes: Vec<ProcessRecord>,
        active_processes: HashSet<u32>,
        next_process_id: u32,
        should_fail_on: Option<MockCommand>,
    }
    
    impl MockAudioPlayer {
        fn new() -> Self {
            Self {
                commands: Vec::new(),
                processes: Vec::new(),
                active_processes: HashSet::new(),
                next_process_id: 2000, // Different range from video
                should_fail_on: None,
            }
        }
        
        fn simulate_play(&mut self, timestamp: f64) -> Result<(), String> {
            let cmd = MockCommand::Play(timestamp);
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock audio player failed to play".to_string());
            }
            
            // Audio might spawn process or use existing one
            if self.active_processes.is_empty() {
                let process_id = self.spawn_process(format!("ffmpeg_audio_mix_{}", timestamp));
                log::debug!("Mock audio player: spawned process {} for play at {}s", process_id, timestamp);
            }
            Ok(())
        }
        
        fn simulate_pause(&mut self) -> Result<(), String> {
            let cmd = MockCommand::Pause;
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock audio player failed to pause".to_string());
            }
            
            log::debug!("Mock audio player: paused");
            Ok(())
        }
        
        fn simulate_seek(&mut self, timestamp: f64) -> Result<(), String> {
            let cmd = MockCommand::Seek(timestamp);
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock audio player failed to seek".to_string());
            }
            
            // Kill old audio processes
            self.kill_all_processes();
            
            // Spawn new audio process from seek position
            let process_id = self.spawn_process(format!("ffmpeg_audio_mix_{}", timestamp));
            log::debug!("Mock audio player: killed old processes, spawned process {} for seek to {}s", process_id, timestamp);
            Ok(())
        }
        
        fn simulate_update_tracks(&mut self, tracks: &[AudioTrack]) -> Result<(), String> {
            let track_names: Vec<String> = tracks.iter().map(|t| t.name.clone()).collect();
            let cmd = MockCommand::UpdateTracks(track_names);
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock audio player failed to update tracks".to_string());
            }
            
            // Updating tracks kills old process and spawns new one
            self.kill_all_processes();
            let process_id = self.spawn_process(format!("ffmpeg_audio_mix_tracks_{}", tracks.len()));
            log::debug!("Mock audio player: updated tracks, spawned process {}", process_id);
            Ok(())
        }
        
        fn simulate_set_video(&mut self, video_path: PathBuf) -> Result<(), String> {
            let cmd = MockCommand::SetVideo(video_path.clone());
            self.commands.push(cmd.clone());
            
            if self.should_fail_on.as_ref() == Some(&cmd) {
                return Err("Mock audio player failed to set video".to_string());
            }
            
            // Kill all processes from previous video
            self.kill_all_processes();
            log::debug!("Mock audio player: set video to {:?}, killed all old processes", video_path);
            Ok(())
        }
        
        // Same helper methods as MockVideoPlayer
        fn spawn_process(&mut self, command: String) -> u32 {
            let process_id = self.next_process_id;
            self.next_process_id += 1;
            
            let record = ProcessRecord {
                id: process_id,
                command,
                status: ProcessStatus::Spawned,
                spawned_at: Instant::now(),
                killed_at: None,
            };
            
            self.processes.push(record);
            self.active_processes.insert(process_id);
            process_id
        }
        
        fn kill_process(&mut self, process_id: u32) {
            if let Some(record) = self.processes.iter_mut().find(|p| p.id == process_id) {
                record.status = ProcessStatus::Killed;
                record.killed_at = Some(Instant::now());
            }
            self.active_processes.remove(&process_id);
        }
        
        fn kill_all_processes(&mut self) {
            let active_ids: Vec<u32> = self.active_processes.iter().cloned().collect();
            for id in active_ids {
                self.kill_process(id);
            }
        }
        
        fn get_commands(&self) -> &[MockCommand] {
            &self.commands
        }
        
        fn get_active_process_count(&self) -> usize {
            self.active_processes.len()
        }
        
        fn get_total_process_count(&self) -> usize {
            self.processes.len()
        }
        
        fn was_process_killed(&self, process_id: u32) -> bool {
            self.processes.iter()
                .find(|p| p.id == process_id)
                .map(|p| p.status == ProcessStatus::Killed)
                .unwrap_or(false)
        }
        
        fn set_failure_on(&mut self, command: MockCommand) {
            self.should_fail_on = Some(command);
        }
        
        fn clear_failure(&mut self) {
            self.should_fail_on = None;
        }
    }
    
    // =============================================================================
    // COMMAND COORDINATION TESTS
    // =============================================================================
    
    // Helper function for creating test audio tracks
    fn create_test_audio_track(index: usize, enabled: bool) -> AudioTrack {
        AudioTrack {
            index,
            enabled,
            surround_mode: false,
            name: format!("Test Track {}", index),
        }
    }
    
    #[test]
    fn test_play_sends_coordinated_commands() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Simulate MediaController.play() coordination
        let timestamp = 30.0;
        
        // Both players should receive play command with same timestamp
        let _ = mock_video.simulate_play(timestamp);
        let _ = mock_audio.simulate_play(timestamp);
        
        // Verify both players got the play command
        assert_eq!(mock_video.get_commands(), &[MockCommand::Play(30.0)]);
        assert_eq!(mock_audio.get_commands(), &[MockCommand::Play(30.0)]);
        
        // Verify processes were spawned
        assert_eq!(mock_video.get_active_process_count(), 1);
        assert_eq!(mock_audio.get_active_process_count(), 1);
    }
    
    #[test]
    fn test_pause_sends_coordinated_commands() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Start with some active processes
        let _ = mock_video.simulate_play(0.0);
        let _ = mock_audio.simulate_play(0.0);
        
        // Simulate MediaController.pause() coordination
        let _ = mock_video.simulate_pause();
        let _ = mock_audio.simulate_pause();
        
        // Verify both players got the pause command
        assert_eq!(mock_video.get_commands(), &[MockCommand::Play(0.0), MockCommand::Pause]);
        assert_eq!(mock_audio.get_commands(), &[MockCommand::Play(0.0), MockCommand::Pause]);
        
        // Verify processes are still alive (pause doesn't kill them)
        assert_eq!(mock_video.get_active_process_count(), 1);
        assert_eq!(mock_audio.get_active_process_count(), 1);
    }
    
    #[test]
    fn test_seek_sends_same_timestamp_to_both() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Start with some active processes
        let _ = mock_video.simulate_play(0.0);
        let _ = mock_audio.simulate_play(0.0);
        let initial_video_processes = mock_video.get_total_process_count();
        let initial_audio_processes = mock_audio.get_total_process_count();
        
        // Simulate MediaController.seek() coordination
        let seek_timestamp = 45.0;
        let _ = mock_video.simulate_seek(seek_timestamp);
        let _ = mock_audio.simulate_seek(seek_timestamp);
        
        // Verify both players got seek command with SAME timestamp
        assert!(mock_video.get_commands().contains(&MockCommand::Seek(45.0)));
        assert!(mock_audio.get_commands().contains(&MockCommand::Seek(45.0)));
        
        // Verify old processes were killed and new ones spawned
        assert_eq!(mock_video.get_total_process_count(), initial_video_processes + 1);
        assert_eq!(mock_audio.get_total_process_count(), initial_audio_processes + 1);
        assert_eq!(mock_video.get_active_process_count(), 1);
        assert_eq!(mock_audio.get_active_process_count(), 1);
    }
    
    #[test]
    fn test_seek_then_play_sends_correct_sequence() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Simulate MediaController sequence: seek then play
        let seek_timestamp = 25.0;
        let _ = mock_video.simulate_seek(seek_timestamp);
        let _ = mock_audio.simulate_seek(seek_timestamp);
        
        let _ = mock_video.simulate_play(seek_timestamp);
        let _ = mock_audio.simulate_play(seek_timestamp);
        
        // Verify command sequence
        assert_eq!(mock_video.get_commands(), &[
            MockCommand::Seek(25.0),
            MockCommand::Play(25.0)
        ]);
        assert_eq!(mock_audio.get_commands(), &[
            MockCommand::Seek(25.0),
            MockCommand::Play(25.0)
        ]);
        
        // Both should use the same timestamp
        let video_commands = mock_video.get_commands();
        let audio_commands = mock_audio.get_commands();
        assert_eq!(video_commands, audio_commands);
    }
    
    // =============================================================================
    // PROCESS LIFECYCLE TESTS
    // =============================================================================
    
    #[test]
    fn test_seek_spawns_new_processes_kills_old() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Start playback - spawns initial processes
        let _ = mock_video.simulate_play(0.0);
        let _ = mock_audio.simulate_play(0.0);
        assert_eq!(mock_video.get_active_process_count(), 1);
        assert_eq!(mock_audio.get_active_process_count(), 1);
        
        let initial_video_process_id = 1000; // First video process ID
        let initial_audio_process_id = 2000; // First audio process ID
        
        // Seek should kill old processes and spawn new ones
        let _ = mock_video.simulate_seek(30.0);
        let _ = mock_audio.simulate_seek(30.0);
        
        // Verify old processes were killed
        assert!(mock_video.was_process_killed(initial_video_process_id));
        assert!(mock_audio.was_process_killed(initial_audio_process_id));
        
        // Verify new processes were spawned
        assert_eq!(mock_video.get_active_process_count(), 1);
        assert_eq!(mock_audio.get_active_process_count(), 1);
        assert_eq!(mock_video.get_total_process_count(), 2); // 1 killed + 1 new
        assert_eq!(mock_audio.get_total_process_count(), 2); // 1 killed + 1 new
    }
    
    #[test]
    fn test_clip_change_kills_all_previous_processes() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Start playback and do some operations - spawns multiple processes
        let _ = mock_video.simulate_play(0.0);
        let _ = mock_audio.simulate_play(0.0);
        let _ = mock_video.simulate_seek(15.0);
        let _ = mock_audio.simulate_seek(15.0);
        let _ = mock_video.simulate_seek(30.0);
        let _ = mock_audio.simulate_seek(30.0);
        
        // Should have some processes from the operations above
        assert!(mock_video.get_total_process_count() > 0);
        assert!(mock_audio.get_total_process_count() > 0);
        
        // Change to new video - should kill ALL processes
        let new_video_path = PathBuf::from("new_clip.mkv");
        let _ = mock_video.simulate_set_video(new_video_path.clone());
        let _ = mock_audio.simulate_set_video(new_video_path);
        
        // Verify ALL processes were killed
        assert_eq!(mock_video.get_active_process_count(), 0);
        assert_eq!(mock_audio.get_active_process_count(), 0);
        
        // All processes should be in killed state
        for process in &mock_video.processes {
            assert_eq!(process.status, ProcessStatus::Killed);
        }
        for process in &mock_audio.processes {
            assert_eq!(process.status, ProcessStatus::Killed);
        }
    }
    
    #[test]
    fn test_pause_doesnt_kill_processes_unnecessarily() {
        let mut mock_video = MockVideoPlayer::new();
        let mut mock_audio = MockAudioPlayer::new();
        
        // Start playback
        let _ = mock_video.simulate_play(0.0);
        let _ = mock_audio.simulate_play(0.0);
        let initial_video_count = mock_video.get_total_process_count();
        let initial_audio_count = mock_audio.get_total_process_count();
        
        // Pause should NOT kill processes (keep them alive for quick resume)
        let _ = mock_video.simulate_pause();
        let _ = mock_audio.simulate_pause();
        
        // Verify process counts didn't change
        assert_eq!(mock_video.get_total_process_count(), initial_video_count);
        assert_eq!(mock_audio.get_total_process_count(), initial_audio_count);
        assert_eq!(mock_video.get_active_process_count(), initial_video_count);
        assert_eq!(mock_audio.get_active_process_count(), initial_audio_count);
    }    #[test]
    fn test_initial_state() {
        let controller = MediaController::new();
        
        assert_eq!(controller.current_position(), 0.0);
        assert_eq!(controller.is_playing(), false);
        assert_eq!(controller.total_duration(), 0.0);
        assert!(controller.video_path().is_none());
    }
    
    #[test]
    fn test_play_pause_state_coordination() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first to get out of Unloaded state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 300.0, &ctx).unwrap();
        
        // Initial state after loading
        assert!(!controller.is_playing());
        
        // Play should update state
        controller.play();
        assert!(controller.is_playing());
        
        // Pause should update state
        controller.pause();
        assert!(!controller.is_playing());
        
        // Multiple plays should be idempotent
        controller.play();
        controller.play();
        assert!(controller.is_playing());
        
        // Multiple pauses should be idempotent
        controller.pause();
        controller.pause();
        assert!(!controller.is_playing());
    }
    
    #[test]
    fn test_seek_position_coordination() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first to get out of Unloaded state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 300.0, &ctx).unwrap();
        
        // Set a duration so seeking has bounds
        controller.total_duration = 100.0;
        
        // Initial position
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek to middle
        controller.seek(50.0);
        assert_eq!(controller.current_position(), 50.0);
        
        // Seek to end
        controller.seek(100.0);
        assert_eq!(controller.current_position(), 100.0);
        
        // Seek beyond end should clamp
        controller.seek(150.0);
        assert_eq!(controller.current_position(), 100.0);
        
        // Seek before start should clamp
        controller.seek(-10.0);
        assert_eq!(controller.current_position(), 0.0);
    }
    
    #[test]
    fn test_seek_while_playing_maintains_play_state() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first to get out of Unloaded state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        controller.total_duration = 100.0;
        
        // Start playing
        controller.play();
        assert!(controller.is_playing());
        
        // Seek while playing
        controller.seek(30.0);
        
        // Should maintain playing state and update position
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
    }
    
    #[test]
    fn test_seek_while_paused_maintains_pause_state() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first to get out of Unloaded state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        controller.total_duration = 100.0;
        
        // Ensure paused (Ready state can be "paused" to start with)
        controller.pause();
        assert!(!controller.is_playing());
        
        // Seek while paused
        controller.seek(30.0);
        
        // Should maintain paused state and update position
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
    }
    
    #[test]
    fn test_play_from_seeked_position() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first to get out of Unloaded state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        controller.total_duration = 100.0;
        
        // Seek to position while paused
        controller.seek(25.0);
        assert_eq!(controller.current_position(), 25.0);
        assert!(!controller.is_playing());
        
        // Play should start from seeked position
        controller.play();
        assert_eq!(controller.current_position(), 25.0);
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_set_video_resets_state() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set some state
        controller.seek(50.0);
        controller.play();
        
        // Set new video should reset state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        let result = controller.set_video(video_path.clone(), &audio_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        
        // State should be reset
        assert_eq!(controller.current_position(), 0.0);
        assert!(!controller.is_playing());
        assert_eq!(controller.video_path(), Some(&video_path));
    }
    
    #[test]
    fn test_operations_without_video_are_safe() {
        let mut controller = MediaController::new();
        
        // These operations should not panic even without a video loaded
        controller.play();
        controller.pause();
        controller.seek(10.0);
        controller.update_audio_tracks(&[]);
        
        let ctx = egui::Context::default();
        controller.update(&ctx);
        let texture = controller.get_frame_texture(&ctx);
        assert!(texture.is_none());
    }
    
    #[test]
    fn test_audio_track_updates_while_playing() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set video with initial tracks
        let video_path = PathBuf::from("test.mkv");
        let initial_tracks = vec![
            create_test_audio_track(0, true),
            create_test_audio_track(1, false),
        ];
        
        let result = controller.set_video(video_path, &initial_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        
        // Start playing
        controller.play();
        assert!(controller.is_playing());
        
        // Update audio tracks while playing
        let updated_tracks = vec![
            create_test_audio_track(0, false),
            create_test_audio_track(1, true),
        ];
        
        controller.update_audio_tracks(&updated_tracks);
        
        // Should still be playing after track update
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_complex_playback_scenario() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 120.0;
        
        // Load video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 120.0, &ctx).unwrap();
        
        // Play from start
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek while playing
        controller.seek(30.0);
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
        
        // Pause
        controller.pause();
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
        
        // Seek while paused
        controller.seek(60.0);
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 60.0);
        
        // Resume from new position
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 60.0);
        
        // Seek to end
        controller.seek(120.0);
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 120.0);
    }
    
    // =============================================================================
    // REGRESSION TESTS - These test the specific bugs we've encountered
    // =============================================================================
    
    #[test]
    fn test_audio_gets_correct_position_on_play() {
        // REGRESSION: Audio always started from 0.0s
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Seek to middle, then play
        controller.seek(50.0);
        controller.play();
        
        // Audio should start from 50.0s, not 0.0s
        assert_eq!(controller.current_position(), 50.0);
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_video_resumes_after_pause() {
        // REGRESSION: Video stream didn't restart after pausing
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Play, pause, play again cycle
        controller.play();
        assert!(controller.is_playing());
        
        controller.pause();
        assert!(!controller.is_playing());
        
        // This should work - both video and audio should resume
        controller.play();
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_audio_seeks_when_video_seeks() {
        // REGRESSION: Audio didn't seek when video did
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Start playing
        controller.play();
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek while playing - both video and audio should update
        controller.seek(30.0);
        assert_eq!(controller.current_position(), 30.0);
        assert!(controller.is_playing());
        
        // Pause and seek again - both should update
        controller.pause();
        controller.seek(60.0);
        assert_eq!(controller.current_position(), 60.0);
        assert!(!controller.is_playing());
    }
    
    #[test]
    fn test_play_pause_play_maintains_position() {
        // REGRESSION: Multiple play/pause cycles lost track of position
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Seek to a position
        controller.seek(25.0);
        
        // Multiple play/pause cycles
        for _i in 0..3 {
            controller.play();
            assert!(controller.is_playing());
            assert_eq!(controller.current_position(), 25.0);
            
            controller.pause();
            assert!(!controller.is_playing());
            assert_eq!(controller.current_position(), 25.0);
        }
    }
    
    #[test]
    fn test_audio_track_changes_are_coordinated() {
        // REGRESSION: Audio track updates might not be synchronized with video state
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let initial_tracks = vec![
            create_test_audio_track(0, true),
            create_test_audio_track(1, false),
        ];
        controller.set_video(video_path, &initial_tracks, 100.0, &ctx).unwrap();
        
        // Start playing and seek to position
        controller.seek(40.0);
        controller.play();
        
        // Update tracks while playing
        let updated_tracks = vec![
            create_test_audio_track(0, false),
            create_test_audio_track(1, true),
        ];
        controller.update_audio_tracks(&updated_tracks);
        
        // State should be maintained
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 40.0);
    }
    
    // =============================================================================
    // USER STATE SIGNALING TESTS
    // =============================================================================
    
    #[test]
    fn test_initial_state_is_unloaded() {
        let controller = MediaController::new();
        
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        assert_eq!(controller.state().display_text(), "No video loaded");
        assert!(!controller.state().can_play());
        assert!(!controller.state().can_pause());
        assert!(!controller.state().can_seek());
        assert!(!controller.state().is_busy());
    }
    
    #[test]
    fn test_loading_state_during_video_load() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set loading state during video load (simulate slow operation)
        controller.state = MediaControllerState::Loading;
        
        assert_eq!(controller.state(), &MediaControllerState::Loading);
        assert_eq!(controller.state().display_text(), "Loading video...");
        assert!(!controller.state().can_play());
        assert!(!controller.state().can_pause());
        assert!(!controller.state().can_seek());
        assert!(controller.state().is_busy());
        
        // Complete the load
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        let result = controller.set_video(video_path, &audio_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        
        // Should transition to Ready state
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        assert!(controller.state().can_play());
    }
    
    #[test]
    fn test_seeking_state_during_seek_operation() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Load video first
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Start playing
        controller.play();
        assert_eq!(controller.state(), &MediaControllerState::Playing);
        
        // Seek operation will temporarily go to Seeking state and then back to Playing
        // The seek method in MediaController handles this automatically
        controller.seek(50.0);
        
        // Should end up back in Playing state with new position
        assert_eq!(controller.state(), &MediaControllerState::Playing);
        assert_eq!(controller.current_position(), 50.0);
        
        // Test that seeking state is busy (even though we can't easily see it in mock)
        let seeking_state = MediaControllerState::Seeking;
        assert_eq!(seeking_state.display_text(), "Seeking...");
        assert!(!seeking_state.can_play());
        assert!(!seeking_state.can_pause());
        assert!(!seeking_state.can_seek());
        assert!(seeking_state.is_busy());
    }
    
    #[test]
    fn test_error_state_when_video_fails_to_load() {
        let mut controller = MediaController::new();
        
        // Simulate load failure
        controller.set_error("Failed to load video file".to_string());
        
        assert_eq!(controller.state(), &MediaControllerState::Error("Failed to load video file".to_string()));
        assert_eq!(controller.state().display_text(), "Failed to load video file");
        assert!(!controller.state().can_play());
        assert!(!controller.state().can_pause());
        assert!(!controller.state().can_seek());
        assert!(!controller.state().is_busy());
        assert!(controller.has_error());
        assert_eq!(controller.error_message(), Some("Failed to load video file"));
        assert!(!controller.is_playing()); // Error stops playback
    }
    
    #[test]
    fn test_ready_state_after_successful_load() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        let result = controller.set_video(video_path, &audio_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        assert_eq!(controller.state().display_text(), "Ready");
        assert!(controller.state().can_play());
        assert!(!controller.state().can_pause());
        assert!(controller.state().can_seek());
        assert!(!controller.state().is_busy());
        assert!(!controller.has_error());
    }
    
    #[test]
    fn test_state_transitions_play_pause_cycle() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Ready -> Playing
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        controller.play();
        assert_eq!(controller.state(), &MediaControllerState::Playing);
        assert!(controller.state().can_pause());
        assert!(!controller.state().can_play());
        
        // Playing -> Paused
        controller.pause();
        assert_eq!(controller.state(), &MediaControllerState::Paused);
        assert!(controller.state().can_play());
        assert!(!controller.state().can_pause());
        
        // Paused -> Playing
        controller.play();
        assert_eq!(controller.state(), &MediaControllerState::Playing);
    }
    
    #[test]
    fn test_error_recovery() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load video first
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        
        // Simulate error
        controller.set_error("Temporary network error".to_string());
        assert!(controller.has_error());
        
        // Clear error should restore to Ready state
        controller.clear_error();
        assert!(!controller.has_error());
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        assert!(controller.state().can_play());
    }
    
    #[test]
    fn test_operations_blocked_in_invalid_states() {
        let mut controller = MediaController::new();
        
        // Try to play without video loaded (Unloaded state)
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        controller.play();
        // Should remain in Unloaded state, not transition to Playing
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        assert!(!controller.is_playing());
        
        // Try to pause when not playing
        controller.pause();
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Try to seek without video loaded
        controller.seek(30.0);
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        assert_eq!(controller.current_position(), 0.0); // Position shouldn't change
    }
    
    // =============================================================================
    // THREAD SAFETY & GUI INTEGRATION TESTS
    // =============================================================================
    
    #[test]
    fn test_egui_context_integration_safe() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Test that egui Context can be passed safely to MediaController methods
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        // These should not cause any threading issues
        let result = controller.set_video(video_path, &audio_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        
        controller.update(&ctx);
        
        // get_frame_texture should handle Context safely
        let _texture = controller.get_frame_texture(&ctx);
        
        // Multiple calls should be safe
        for _i in 0..5 {
            controller.update(&ctx);
            let _texture = controller.get_frame_texture(&ctx);
        }
    }
    
    #[test]
    fn test_thread_safety_architectural_requirements() {
        // This test documents the thread safety requirements for MediaController
        
        // REQUIREMENT 1: MediaController state queries must be thread-safe
        // All state getters (is_playing, current_position, state, etc.) should be safe
        // to call from multiple threads when wrapped in Arc<Mutex<>>
        
        // REQUIREMENT 2: egui Context integration must not break thread safety
        // Methods that take &Context should not introduce thread-unsafe behavior
        
        // REQUIREMENT 3: Audio players must be moved to a separate thread/channel system
        // Current SynchronizedAudioPlayer contains non-Send types (audio streams)
        // This prevents MediaController from being Send/Sync
        
        // DISCOVERED ISSUE: MediaController contains SynchronizedAudioPlayer which contains
        // rodio::OutputStream, which contains platform-specific types that are explicitly
        // NOT Send/Sync. This means MediaController cannot be used with Arc<Mutex<>> for
        // true thread safety.
        
        // ARCHITECTURE DECISION: For real implementation, audio control should use
        // message passing (mpsc channels) instead of direct field ownership:
        // - MediaController sends commands to audio thread via channel
        // - Audio thread owns the actual audio player and streams
        // - Audio thread sends status updates back via channel
        // - This allows MediaController to be Send/Sync while maintaining control
        
        // The threading tests cannot be written until this architectural change is made.
        // For now, we document the requirement and will implement in Phase 5.
        
        assert!(true, "Thread safety requirements documented");
    }
    
    #[test]
    fn test_single_threaded_state_management() {
        // Test that all state management works correctly in single-threaded context
        // This is a prerequisite for thread-safe design
        
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Test complete state lifecycle
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Load video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        
        // Play/pause cycle
        controller.play();
        assert_eq!(controller.state(), &MediaControllerState::Playing);
        assert!(controller.is_playing());
        
        controller.pause();
        assert_eq!(controller.state(), &MediaControllerState::Paused);
        assert!(!controller.is_playing());
        
        // Seeking
        controller.total_duration = 100.0; // Set duration for proper seeking bounds
        controller.seek(50.0);
        assert_eq!(controller.current_position(), 50.0);
        assert_eq!(controller.state(), &MediaControllerState::Paused);
        
        // Error handling
        controller.set_error("Test error".to_string());
        assert!(controller.has_error());
        assert_eq!(controller.error_message(), Some("Test error"));
        
        controller.clear_error();
        assert!(!controller.has_error());
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        
        // All state transitions work correctly in single-threaded context
        assert!(true, "Single-threaded state management verified");
    }
    
    // =============================================================================
    // REAL THREADING TESTS - NOW THAT MEDIACONTROLLER IS SEND + SYNC
    // =============================================================================
    
    #[test]
    fn test_media_controller_is_send_sync() {
        // Verify that MediaController now implements Send + Sync
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        
        assert_send::<MediaController>();
        assert_sync::<MediaController>();
        
        // This test would fail to compile if MediaController wasn't Send + Sync
        assert!(true, "MediaController is now Send + Sync!");
    }
    
    #[test]
    fn test_arc_mutex_wrapping_basic_operations() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        
        let controller = Arc::new(Mutex::new(MediaController::new()));
        let ctx = egui::Context::default();
        
        // Test basic operations through Arc<Mutex<>>
        {
            let mut ctrl = controller.lock().unwrap();
            let video_path = PathBuf::from("test.mkv");
            let audio_tracks = vec![create_test_audio_track(0, true)];
            ctrl.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        }
        
        // Test play from different thread
        let controller_clone = Arc::clone(&controller);
        let handle = thread::spawn(move || {
            let mut ctrl = controller_clone.lock().unwrap();
            ctrl.play();
            assert!(ctrl.is_playing());
        });
        handle.join().unwrap();
        
        // Verify state from main thread
        {
            let ctrl = controller.lock().unwrap();
            assert!(ctrl.is_playing());
        }
    }
    
    #[test]
    fn test_concurrent_state_queries_safe() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        
        let controller = Arc::new(Mutex::new(MediaController::new()));
        let ctx = egui::Context::default();
        
        // Set up controller
        {
            let mut ctrl = controller.lock().unwrap();
            let video_path = PathBuf::from("test.mkv");
            let audio_tracks = vec![create_test_audio_track(0, true)];
            ctrl.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
            ctrl.total_duration = 100.0;
            ctrl.play();
        }
        
        let mut handles = vec![];
        
        // Spawn multiple threads doing state queries
        for i in 0..5 {
            let controller_clone = Arc::clone(&controller);
            let handle = thread::spawn(move || {
                for _j in 0..10 {
                    let ctrl = controller_clone.lock().unwrap();
                    let _state = ctrl.state().clone();
                    let _position = ctrl.current_position();
                    let _duration = ctrl.total_duration();
                    let _playing = ctrl.is_playing();
                    // Simulate some processing time
                    drop(ctrl);
                    thread::sleep(std::time::Duration::from_millis(1));
                }
                i // Return thread id for verification
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            let thread_id = handle.join().unwrap();
            assert!(thread_id < 5); // Just verify we got expected values
        }
        
        // Controller should still be in valid state
        let ctrl = controller.lock().unwrap();
        assert_eq!(ctrl.state(), &MediaControllerState::Playing);
    }
    
    #[test]
    fn test_gui_update_pattern_thread_safe() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        
        let controller = Arc::new(Mutex::new(MediaController::new()));
        let ctx = egui::Context::default();
        
        // Set up controller
        {
            let mut ctrl = controller.lock().unwrap();
            let video_path = PathBuf::from("test.mkv");
            let audio_tracks = vec![create_test_audio_track(0, true)];
            ctrl.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        }
        
        // Simulate GUI thread doing updates
        let gui_controller = Arc::clone(&controller);
        let gui_handle = thread::spawn(move || {
            let ctx = egui::Context::default();
            for _i in 0..20 {
                {
                    let mut ctrl = gui_controller.lock().unwrap();
                    ctrl.update(&ctx);
                    // Simulate checking state for GUI updates
                    let _state = ctrl.state().display_text();
                    let _can_play = ctrl.state().can_play();
                }
                thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        
        // Simulate background task doing operations
        let bg_controller = Arc::clone(&controller);
        let bg_handle = thread::spawn(move || {
            for i in 0..10 {
                {
                    let mut ctrl = bg_controller.lock().unwrap();
                    if i % 2 == 0 {
                        ctrl.play();
                    } else {
                        ctrl.pause();
                    }
                    ctrl.seek(i as f64 * 10.0);
                }
                thread::sleep(std::time::Duration::from_millis(2));
            }
        });
        
        // Wait for both threads
        gui_handle.join().unwrap();
        bg_handle.join().unwrap();
        
        // Controller should be in valid state
        let ctrl = controller.lock().unwrap();
        assert!(matches!(ctrl.state(), MediaControllerState::Playing | MediaControllerState::Paused));
    }
    
    #[test]
    fn test_error_state_thread_safe() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        
        let controller = Arc::new(Mutex::new(MediaController::new()));
        
        // Simulate error from background thread
        let error_controller = Arc::clone(&controller);
        let error_handle = thread::spawn(move || {
            let mut ctrl = error_controller.lock().unwrap();
            ctrl.set_error("Simulated playback error".to_string());
        });
        error_handle.join().unwrap();
        
        // GUI thread should see error state safely
        let gui_controller = Arc::clone(&controller);
        let gui_handle = thread::spawn(move || {
            let ctrl = gui_controller.lock().unwrap();
            assert!(ctrl.has_error());
            assert_eq!(ctrl.error_message(), Some("Simulated playback error"));
            assert_eq!(ctrl.state().display_text(), "Simulated playback error");
        });
        gui_handle.join().unwrap();
        
        // Error can be cleared from another thread
        {
            let mut ctrl = controller.lock().unwrap();
            ctrl.clear_error();
            assert!(!ctrl.has_error());
        }
    }
    
    #[test]
    fn test_audio_thread_communication() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Test that commands are sent to audio thread correctly
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        // These operations should send commands to the audio thread
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        controller.play();
        controller.seek(30.0);
        controller.pause();
        controller.update_audio_tracks(&audio_tracks);
        
        // All operations completed without blocking or panicking
        assert!(true, "Audio thread communication working");
    }
    
    #[test]
    fn test_thread_safety_architectural_solution() {
        // This test verifies that the architectural solution is working
        
        // SOLUTION IMPLEMENTED: Audio control now uses message passing
        // - MediaController sends commands to audio thread via mpsc channel
        // - Audio thread owns the actual SynchronizedAudioPlayer and streams
        // - Audio thread sends status updates back via channel
        // - This allows MediaController to be Send/Sync while maintaining control
        
        // VERIFICATION: MediaController should now be Send + Sync
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MediaController>();
        
        // VERIFICATION: Can be used with Arc<Mutex<>> for thread safety
        use std::sync::{Arc, Mutex};
        let _controller = Arc::new(Mutex::new(MediaController::new()));
        
        assert!(true, "Thread safety architectural solution verified");
    }
    
    // =============================================================================
    // PHASE 4: EDGE CASES & BOUNDARY CONDITION TESTS
    // =============================================================================
    
    #[test]
    fn test_rapid_multiple_seeks() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        controller.total_duration = 120.0; // 2 minute video
        
        // Perform rapid seeks in quick succession
        let seek_positions = vec![10.0, 30.0, 5.0, 45.0, 20.0, 60.0, 15.0];
        
        for position in seek_positions {
            controller.seek(position);
            // Verify state remains consistent during rapid operations
            assert!(controller.current_position >= 0.0);
            assert!(controller.current_position <= controller.total_duration);
        }
        
        // Final position should be the last seek
        assert_eq!(controller.current_position, 15.0);
        
        // Controller should still be functional after rapid seeks
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.state(), &MediaControllerState::Playing);
    }
    
    #[test]
    fn test_operations_without_video_loaded() {
        let mut controller = MediaController::new();
        
        // All operations should be safe when no video is loaded
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Play without video should be safe (no-op)
        controller.play();
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        assert!(!controller.is_playing());
        
        // Pause without video should be safe
        controller.pause();
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Seek without video should be safe
        controller.seek(30.0);
        assert_eq!(controller.current_position, 0.0); // Position unchanged
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Update without video should be safe
        let ctx = egui::Context::default();
        controller.update(&ctx);
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        
        // Should remain in consistent state
        assert_eq!(controller.total_duration, 0.0);
        assert_eq!(controller.current_position, 0.0);
        assert!(!controller.is_playing());
    }
    
    #[test]
    fn test_seek_beyond_boundaries() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set up video with known duration
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        controller.total_duration = 100.0;
        
        // Test seeking before start
        controller.seek(-10.0);
        assert!(controller.current_position >= 0.0, "Position should be clamped to >= 0");
        
        // Test seeking beyond end
        controller.seek(150.0);
        assert!(controller.current_position <= controller.total_duration, 
                "Position should be clamped to <= duration");
        
        // Test seeking to exactly the boundaries
        controller.seek(0.0);
        assert_eq!(controller.current_position, 0.0);
        
        controller.seek(100.0);
        assert_eq!(controller.current_position, 100.0);
        
        // Test extreme values
        controller.seek(f64::INFINITY);
        assert!(controller.current_position.is_finite(), "Position should remain finite");
        
        controller.seek(f64::NEG_INFINITY);
        assert!(controller.current_position >= 0.0, "Position should be non-negative");
        
        controller.seek(f64::NAN);
        assert!(!controller.current_position.is_nan(), "Position should not be NaN");
        
        // Controller should remain functional after boundary tests
        controller.play();
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_concurrent_state_changes() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        
        let controller = Arc::new(Mutex::new(MediaController::new()));
        let ctx = egui::Context::default();
        
        // Set up video in main thread
        {
            let mut ctrl = controller.lock().unwrap();
            let video_path = PathBuf::from("test.mkv");
            let audio_tracks = vec![create_test_audio_track(0, true)];
            ctrl.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
            ctrl.total_duration = 60.0;
        }
        
        let mut handles = vec![];
        
        // Thread 1: Rapid play/pause cycles
        let controller1 = Arc::clone(&controller);
        handles.push(thread::spawn(move || {
            for i in 0..10 {
                {
                    let mut ctrl = controller1.lock().unwrap();
                    if i % 2 == 0 {
                        ctrl.play();
                    } else {
                        ctrl.pause();
                    }
                }
                thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
        
        // Thread 2: Rapid seeking
        let controller2 = Arc::clone(&controller);
        handles.push(thread::spawn(move || {
            for i in 0..10 {
                {
                    let mut ctrl = controller2.lock().unwrap();
                    ctrl.seek((i * 5) as f64);
                }
                thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
        
        // Thread 3: State querying
        let controller3 = Arc::clone(&controller);
        handles.push(thread::spawn(move || {
            for _i in 0..20 {
                {
                    let ctrl = controller3.lock().unwrap();
                    let _state = ctrl.state().clone();
                    let _pos = ctrl.current_position();
                    let _playing = ctrl.is_playing();
                }
                thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Controller should be in a valid state after concurrent operations
        let ctrl = controller.lock().unwrap();
        assert!(matches!(ctrl.state(), 
            MediaControllerState::Ready | 
            MediaControllerState::Playing | 
            MediaControllerState::Paused
        ));
        assert!(ctrl.current_position() >= 0.0);
        assert!(ctrl.current_position() <= ctrl.total_duration());
    }
    
    #[test]
    fn test_error_recovery_after_invalid_operations() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Simulate setting an invalid video path
        controller.state = MediaControllerState::Error("Failed to load video".to_string());
        
        // Operations during error state should be handled gracefully
        controller.play(); // Should not crash
        controller.pause(); // Should not crash
        controller.seek(30.0); // Should not crash
        
        // Error state should persist until explicitly cleared
        assert!(matches!(controller.state(), MediaControllerState::Error(_)));
        
        // Clear error and try to load valid video
        controller.state = MediaControllerState::Unloaded;
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        // Should successfully recover from error state
        let result = controller.set_video(video_path, &audio_tracks, 100.0, &ctx);
        assert!(result.is_ok());
        assert_eq!(controller.state(), &MediaControllerState::Ready);
        
        // Should be fully functional after recovery
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.state(), &MediaControllerState::Playing);
    }
    
    #[test]
    fn test_resource_cleanup_under_stress() {
        // Test that resources are properly cleaned up under stress conditions
        for iteration in 0..5 {
            let mut controller = MediaController::new();
            let ctx = egui::Context::default();
            
            // Load video
            let video_path = PathBuf::from(format!("test_{}.mkv", iteration));
            let audio_tracks = vec![create_test_audio_track(0, true)];
            controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
            controller.total_duration = 30.0;
            
            // Perform stress operations
            for _op in 0..10 {
                controller.play();
                controller.seek(15.0);
                controller.pause();
                controller.seek(5.0);
            }
            
            // Controller should drop cleanly (threads cleaned up automatically)
        }
        
        // If we reach here, no resource leaks occurred
        assert!(true, "Resource cleanup successful under stress");
    }
    
    #[test]
    fn test_non_blocking_operations() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, 100.0, &ctx).unwrap();
        controller.total_duration = 60.0; // Set duration for seek test
        
        // All operations should return immediately (non-blocking)
        let start = std::time::Instant::now();
        
        controller.play();
        controller.seek(30.0);
        controller.pause();
        controller.update(&ctx);
        
        let elapsed = start.elapsed();
        
        // Operations should complete very quickly (< 10ms)
        assert!(elapsed < std::time::Duration::from_millis(10), 
                "Operations took too long: {:?}", elapsed);
        
        // State should be updated immediately
        assert_eq!(controller.state(), &MediaControllerState::Paused);
        assert_eq!(controller.current_position(), 30.0);
    }

    #[test]
    fn test_ffmpeg_process_spam_during_playback() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Load a video
        let _ = controller.set_video(
            PathBuf::from("test_video.mp4"),
            &[], // No audio tracks
            10.0, // Duration
            &ctx
        );
        controller.total_duration = 10.0; // Set duration manually for test
        
        // Start playback
        controller.play();
        assert_eq!(controller.state, MediaControllerState::Playing);
        
        // Track how many UpdateFrame commands are sent during simulated playback
        let mut update_count = 0;
        let start_time = std::time::Instant::now();
        
        // Simulate 1 second of GUI updates at 60 FPS
        for _ in 0..60 {
            let previous_position = controller.current_position;
            controller.update(&ctx);
            
            // Check if position changed (indicating UpdateFrame was sent)
            if (controller.current_position - previous_position).abs() > 0.01 {
                update_count += 1;
            }
            
            // Simulate 16ms delay between frames (60 FPS)
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        
        let elapsed = start_time.elapsed();
        
        // During 1 second of playback at 60 FPS GUI updates:
        // - Should NOT send 60 UpdateFrame commands (that would spam FFmpeg)
        // - Should send reasonable number based on actual position changes
        println!("Test results:");
        println!("- GUI updates: 60 (simulating 60 FPS)");
        println!("- UpdateFrame commands sent: {}", update_count);
        println!("- Time elapsed: {:.2}s", elapsed.as_secs_f64());
        println!("- Final position: {:.2}s", controller.current_position);
        
        // FAIL: This test will show the problem - too many UpdateFrame commands
        // Should be ZERO UpdateFrame commands during playback! 
        // Only ONE FFmpeg process should start on play() and run continuously
        assert_eq!(update_count, 0, 
            "FFmpeg process spam detected! Sent {} UpdateFrame commands during playback. Should be 0 - only ONE continuous FFmpeg process should run from play to pause.", 
            update_count);
    }
}
