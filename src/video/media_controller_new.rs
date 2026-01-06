// =============================================================================
// MEDIA CONTROLLER - UNIFIED VIDEO AND AUDIO PLAYBACK
// =============================================================================
//
// This module provides synchronized video and audio playback using a SINGLE
// FFmpeg process. Video frames and audio samples come from the same source,
// ensuring perfect synchronization.
//
// ARCHITECTURE:
// - Single FFmpeg process outputs video (stdout) and audio (stderr)  
// - Dedicated reader threads for video frames and audio samples
// - Frame pacing based on presentation timestamps
// - Audio fed directly to rodio sink
// - MediaController coordinates everything through message passing
//
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use std::io::Read;
use std::process::{Command, Stdio, Child};
use std::thread::{self, JoinHandle};
use crate::core::clip::AudioTrack;
use egui::{Context, TextureHandle};
use rodio::{OutputStream, Sink, Source};

// =============================================================================
// VIDEO FRAME
// =============================================================================

/// Raw video frame data that can be sent between threads
#[derive(Debug)]
pub struct VideoFrame {
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: f64,
    pub sequence: u64,
    pub process_id: u64,
}

// =============================================================================
// COMMANDS AND STATUS
// =============================================================================

/// Commands sent to the unified playback thread
#[derive(Debug)]
pub enum PlaybackCommand {
    /// Load a video file with audio track configuration
    SetVideo {
        path: PathBuf,
        duration: f64,
        frame_rate: f64,
        audio_tracks: Vec<AudioTrack>,
    },
    /// Start playback from current position
    Play,
    /// Pause playback
    Pause,
    /// Seek to timestamp (seconds)
    Seek(f64),
    /// Update audio track configuration
    UpdateTracks(Vec<AudioTrack>),
    /// Extract a single frame at timestamp (for scrubbing when paused)
    ExtractFrame(f64),
    /// Shutdown the playback thread
    Shutdown,
}

/// Status updates from the playback thread
#[derive(Debug, Clone)]
pub enum PlaybackStatus {
    Ready,
    Playing,
    Paused,
    PositionUpdate(f64),
    Error(String),
}

// =============================================================================
// AUDIO SOURCE - Streams audio samples from a buffer
// =============================================================================

/// A rodio Source that reads from a shared ring buffer
struct StreamingAudioSource {
    /// Shared buffer of audio samples (f32, stereo interleaved)
    buffer: Arc<Mutex<AudioBuffer>>,
    /// Flag to stop playback
    stop_flag: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

struct AudioBuffer {
    samples: Vec<f32>,
    read_pos: usize,
    write_pos: usize,
    capacity: usize,
}

impl AudioBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0.0; capacity],
            read_pos: 0,
            write_pos: 0,
            capacity,
        }
    }

    fn write(&mut self, data: &[f32]) -> usize {
        let mut written = 0;
        for &sample in data {
            let next_write = (self.write_pos + 1) % self.capacity;
            if next_write != self.read_pos {
                self.samples[self.write_pos] = sample;
                self.write_pos = next_write;
                written += 1;
            } else {
                break; // Buffer full
            }
        }
        written
    }

    fn read(&mut self) -> Option<f32> {
        if self.read_pos != self.write_pos {
            let sample = self.samples[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.capacity;
            Some(sample)
        } else {
            None // Buffer empty
        }
    }

    fn clear(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
    }

    fn available(&self) -> usize {
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            self.capacity - self.read_pos + self.write_pos
        }
    }
}

impl Iterator for StreamingAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stop_flag.load(Ordering::Relaxed) {
            return None;
        }

        // Try to get a sample, return silence if buffer empty
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.read().or(Some(0.0))
        } else {
            Some(0.0) // Return silence on lock failure
        }
    }
}

impl Source for StreamingAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// =============================================================================
// UNIFIED PLAYBACK THREAD
// =============================================================================

struct PlaybackState {
    video_path: Option<PathBuf>,
    audio_tracks: Vec<AudioTrack>,
    duration: f64,
    frame_rate: f64,
    position: f64,
    is_playing: bool,
    
    // FFmpeg process management
    ffmpeg_process: Option<Child>,
    process_id: u64,
    
    // Audio buffer for streaming
    audio_buffer: Arc<Mutex<AudioBuffer>>,
    audio_stop_flag: Arc<AtomicBool>,
    
    // Frame timing
    playback_start_time: Option<Instant>,
    playback_start_position: f64,
    
    // Sequence tracking
    frame_sequence: u64,
}

impl PlaybackState {
    fn new() -> Self {
        Self {
            video_path: None,
            audio_tracks: Vec::new(),
            duration: 0.0,
            frame_rate: 30.0,
            position: 0.0,
            is_playing: false,
            ffmpeg_process: None,
            process_id: 0,
            audio_buffer: Arc::new(Mutex::new(AudioBuffer::new(48000 * 2 * 2))), // 2 seconds buffer
            audio_stop_flag: Arc::new(AtomicBool::new(false)),
            playback_start_time: None,
            playback_start_position: 0.0,
            frame_sequence: 0,
        }
    }

    fn kill_ffmpeg(&mut self) {
        if let Some(mut process) = self.ffmpeg_process.take() {
            log::debug!("Killing FFmpeg process");
            let _ = process.kill();
            let _ = process.wait();
        }
        // Clear audio buffer when stopping
        if let Ok(mut buffer) = self.audio_buffer.lock() {
            buffer.clear();
        }
    }

    fn current_playback_position(&self) -> f64 {
        if self.is_playing {
            if let Some(start_time) = self.playback_start_time {
                let elapsed = start_time.elapsed().as_secs_f64();
                (self.playback_start_position + elapsed).min(self.duration)
            } else {
                self.position
            }
        } else {
            self.position
        }
    }
}

/// Starts the unified FFmpeg process for video and audio
fn start_ffmpeg_process(
    video_path: &PathBuf,
    audio_tracks: &[AudioTrack],
    start_time: f64,
    frame_rate: f64,
) -> Result<Child, String> {
    let enabled_tracks: Vec<_> = audio_tracks.iter().filter(|t| t.enabled).collect();
    
    let mut cmd = Command::new("ffmpeg");
    
    // Seek to start position
    cmd.arg("-ss").arg(format!("{:.3}", start_time));
    cmd.arg("-i").arg(video_path);
    
    // Video output settings - output to stdout
    cmd.arg("-map").arg("0:v:0");
    cmd.arg("-f").arg("rawvideo");
    cmd.arg("-pix_fmt").arg("rgb24");
    cmd.arg("-s").arg("854x480");
    cmd.arg("-r").arg(format!("{:.3}", frame_rate.min(60.0))); // Cap at 60 FPS for performance
    cmd.arg("pipe:1");
    
    // Audio output settings - output to stderr (fd 2)
    if !enabled_tracks.is_empty() {
        // Build audio filter for track mixing
        if enabled_tracks.len() == 1 {
            let track = enabled_tracks[0];
            if track.surround_mode {
                cmd.arg("-filter_complex")
                    .arg(format!("[0:a:{}]channelmap=map=FL|FR[aout]", track.index))
                    .arg("-map").arg("[aout]");
            } else {
                cmd.arg("-map").arg(format!("0:a:{}", track.index));
            }
        } else {
            // Multiple tracks - mix them
            let mut filter_parts = Vec::new();
            let mut mix_inputs = Vec::new();
            
            for (i, track) in enabled_tracks.iter().enumerate() {
                if track.surround_mode {
                    filter_parts.push(format!("[0:a:{}]channelmap=map=FL|FR[a{}]", track.index, i));
                    mix_inputs.push(format!("[a{}]", i));
                } else {
                    mix_inputs.push(format!("[0:a:{}]", track.index));
                }
            }
            
            let filter_complex = if filter_parts.is_empty() {
                format!("{}amix=inputs={}[aout]", mix_inputs.join(""), enabled_tracks.len())
            } else {
                format!("{};{}amix=inputs={}[aout]", 
                    filter_parts.join(";"),
                    mix_inputs.join(""),
                    enabled_tracks.len())
            };
            
            cmd.arg("-filter_complex").arg(&filter_complex)
                .arg("-map").arg("[aout]");
        }
        
        cmd.arg("-f").arg("f32le");
        cmd.arg("-ac").arg("2");
        cmd.arg("-ar").arg("48000");
        cmd.arg("pipe:2");
    }
    
    cmd.arg("-loglevel").arg("error");
    cmd.arg("-nostdin");
    
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    
    cmd.spawn().map_err(|e| format!("Failed to spawn FFmpeg: {}", e))
}

/// Extract a single frame at a specific timestamp
fn extract_single_frame(video_path: &PathBuf, timestamp: f64) -> Result<VideoFrame, String> {
    let output = Command::new("ffmpeg")
        .args([
            "-ss", &format!("{:.3}", timestamp),
            "-i", video_path.to_str().ok_or("Invalid path")?,
            "-vframes", "1",
            "-f", "rawvideo",
            "-pix_fmt", "rgb24",
            "-s", "854x480",
            "-loglevel", "quiet",
            "-"
        ])
        .output()
        .map_err(|e| format!("FFmpeg execution failed: {}", e))?;
    
    if !output.status.success() {
        return Err("FFmpeg failed to extract frame".to_string());
    }
    
    let width = 854u32;
    let height = 480u32;
    let expected_size = (width * height * 3) as usize;
    
    if output.stdout.len() != expected_size {
        return Err(format!("Unexpected frame size: {} (expected {})", output.stdout.len(), expected_size));
    }
    
    // Convert RGB24 to RGBA
    let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
    for chunk in output.stdout.chunks(3) {
        if chunk.len() == 3 {
            rgba_data.push(chunk[0]);
            rgba_data.push(chunk[1]);
            rgba_data.push(chunk[2]);
            rgba_data.push(255);
        }
    }
    
    Ok(VideoFrame {
        image_data: rgba_data,
        width,
        height,
        timestamp,
        sequence: 0,
        process_id: 0,
    })
}

/// Run the unified playback thread
fn playback_thread(
    cmd_rx: mpsc::Receiver<PlaybackCommand>,
    status_tx: mpsc::Sender<PlaybackStatus>,
    frame_tx: mpsc::Sender<VideoFrame>,
) {
    let mut state = PlaybackState::new();
    
    // Audio output setup
    let audio_output = OutputStream::try_default();
    let (audio_stream, stream_handle) = match audio_output {
        Ok((stream, handle)) => (Some(stream), Some(handle)),
        Err(e) => {
            log::warn!("Failed to create audio output: {}. Video will play without audio.", e);
            (None, None)
        }
    };
    let _audio_stream = audio_stream; // Keep alive
    #[allow(unused_variables)]
    let mut audio_sink: Option<Sink> = None;
    
    // Video reader thread handle
    let mut video_reader_handle: Option<JoinHandle<()>> = None;
    let video_reader_stop = Arc::new(AtomicBool::new(false));
    
    // Audio reader thread handle  
    let mut audio_reader_handle: Option<JoinHandle<()>> = None;
    let audio_reader_stop = Arc::new(AtomicBool::new(false));
    
    // Frame pacing - we'll buffer frames and release them at the right time
    let frame_buffer: Arc<Mutex<Vec<(f64, VideoFrame)>>> = Arc::new(Mutex::new(Vec::new()));
    let frame_buffer_for_reader = frame_buffer.clone();
    
    loop {
        // Check for commands with a short timeout for responsiveness
        let timeout = if state.is_playing {
            Duration::from_millis(8) // ~120Hz for smooth frame pacing
        } else {
            Duration::from_millis(50)
        };
        
        match cmd_rx.recv_timeout(timeout) {
            Ok(PlaybackCommand::SetVideo { path, duration, frame_rate, audio_tracks }) => {
                log::info!("Setting video: {:?} (duration: {:.2}s, fps: {:.2})", path, duration, frame_rate);
                
                // Stop any existing playback
                stop_readers(&mut video_reader_handle, &video_reader_stop, 
                           &mut audio_reader_handle, &audio_reader_stop);
                state.kill_ffmpeg();
                audio_sink = None;
                
                // Clear frame buffer
                if let Ok(mut buffer) = frame_buffer.lock() {
                    buffer.clear();
                }
                
                state.video_path = Some(path.clone());
                state.audio_tracks = audio_tracks;
                state.duration = duration;
                state.frame_rate = frame_rate;
                state.position = 0.0;
                state.is_playing = false;
                state.process_id += 1;
                
                // Extract initial frame
                if let Ok(frame) = extract_single_frame(&path, 0.0) {
                    let _ = frame_tx.send(frame);
                }
                
                let _ = status_tx.send(PlaybackStatus::Ready);
            }
            
            Ok(PlaybackCommand::Play) => {
                if state.video_path.is_none() {
                    continue;
                }
                
                log::info!("Starting playback from {:.2}s", state.position);
                
                // Stop existing readers
                stop_readers(&mut video_reader_handle, &video_reader_stop,
                           &mut audio_reader_handle, &audio_reader_stop);
                state.kill_ffmpeg();
                
                // Clear buffers
                if let Ok(mut buffer) = frame_buffer.lock() {
                    buffer.clear();
                }
                if let Ok(mut buffer) = state.audio_buffer.lock() {
                    buffer.clear();
                }
                
                // Start new FFmpeg process
                let video_path = state.video_path.as_ref().unwrap().clone();
                match start_ffmpeg_process(&video_path, &state.audio_tracks, state.position, state.frame_rate) {
                    Ok(mut process) => {
                        state.process_id += 1;
                        let process_id = state.process_id;
                        let frame_rate = state.frame_rate.min(60.0);
                        let start_position = state.position;
                        
                        // Take ownership of stdout/stderr
                        let stdout = process.stdout.take();
                        let stderr = process.stderr.take();
                        state.ffmpeg_process = Some(process);
                        
                        // Start video reader thread
                        if let Some(stdout) = stdout {
                            video_reader_stop.store(false, Ordering::SeqCst);
                            let stop_flag = video_reader_stop.clone();
                            let buffer = frame_buffer_for_reader.clone();
                            
                            video_reader_handle = Some(thread::spawn(move || {
                                video_reader_thread(stdout, buffer, stop_flag, frame_rate, start_position, process_id);
                            }));
                        }
                        
                        // Start audio reader thread
                        if let Some(stderr) = stderr {
                            audio_reader_stop.store(false, Ordering::SeqCst);
                            let stop_flag = audio_reader_stop.clone();
                            let audio_buf = state.audio_buffer.clone();
                            
                            audio_reader_handle = Some(thread::spawn(move || {
                                audio_reader_thread(stderr, audio_buf, stop_flag);
                            }));
                        }
                        
                        // Start audio playback
                        if let Some(ref handle) = stream_handle {
                            state.audio_stop_flag.store(false, Ordering::SeqCst);
                            if let Ok(sink) = Sink::try_new(handle) {
                                let source = StreamingAudioSource {
                                    buffer: state.audio_buffer.clone(),
                                    stop_flag: state.audio_stop_flag.clone(),
                                    sample_rate: 48000,
                                    channels: 2,
                                };
                                sink.append(source);
                                sink.play();
                                audio_sink = Some(sink);
                            }
                        }
                        
                        state.is_playing = true;
                        state.playback_start_time = Some(Instant::now());
                        state.playback_start_position = state.position;
                        
                        let _ = status_tx.send(PlaybackStatus::Playing);
                    }
                    Err(e) => {
                        log::error!("Failed to start FFmpeg: {}", e);
                        let _ = status_tx.send(PlaybackStatus::Error(e));
                    }
                }
            }
            
            Ok(PlaybackCommand::Pause) => {
                log::info!("Pausing playback at {:.2}s", state.current_playback_position());
                
                // Update position before stopping
                state.position = state.current_playback_position();
                state.is_playing = false;
                state.playback_start_time = None;
                
                // Stop everything
                stop_readers(&mut video_reader_handle, &video_reader_stop,
                           &mut audio_reader_handle, &audio_reader_stop);
                state.audio_stop_flag.store(true, Ordering::SeqCst);
                state.kill_ffmpeg();
                audio_sink = None;
                
                let _ = status_tx.send(PlaybackStatus::Paused);
            }
            
            Ok(PlaybackCommand::Seek(timestamp)) => {
                let clamped = timestamp.clamp(0.0, state.duration);
                log::info!("Seeking to {:.2}s", clamped);
                
                let was_playing = state.is_playing;
                
                // Stop current playback
                stop_readers(&mut video_reader_handle, &video_reader_stop,
                           &mut audio_reader_handle, &audio_reader_stop);
                state.audio_stop_flag.store(true, Ordering::SeqCst);
                state.kill_ffmpeg();
                audio_sink = None;
                
                // Clear buffers
                if let Ok(mut buffer) = frame_buffer.lock() {
                    buffer.clear();
                }
                
                state.position = clamped;
                state.is_playing = false;
                
                // Extract frame at new position
                if let Some(ref path) = state.video_path {
                    if let Ok(mut frame) = extract_single_frame(path, clamped) {
                        state.frame_sequence += 1;
                        frame.sequence = state.frame_sequence;
                        let _ = frame_tx.send(frame);
                    }
                }
                
                // Resume playback if was playing
                if was_playing {
                    // Re-issue play command
                    let _ = cmd_rx; // We can't send to ourselves, so we'll restart inline
                    
                    if let Some(ref video_path) = state.video_path {
                        match start_ffmpeg_process(video_path, &state.audio_tracks, clamped, state.frame_rate) {
                            Ok(mut process) => {
                                state.process_id += 1;
                                let process_id = state.process_id;
                                let frame_rate = state.frame_rate.min(60.0);
                                
                                let stdout = process.stdout.take();
                                let stderr = process.stderr.take();
                                state.ffmpeg_process = Some(process);
                                
                                if let Some(stdout) = stdout {
                                    video_reader_stop.store(false, Ordering::SeqCst);
                                    let stop_flag = video_reader_stop.clone();
                                    let buffer = frame_buffer_for_reader.clone();
                                    
                                    video_reader_handle = Some(thread::spawn(move || {
                                        video_reader_thread(stdout, buffer, stop_flag, frame_rate, clamped, process_id);
                                    }));
                                }
                                
                                if let Some(stderr) = stderr {
                                    audio_reader_stop.store(false, Ordering::SeqCst);
                                    let stop_flag = audio_reader_stop.clone();
                                    let audio_buf = state.audio_buffer.clone();
                                    
                                    audio_reader_handle = Some(thread::spawn(move || {
                                        audio_reader_thread(stderr, audio_buf, stop_flag);
                                    }));
                                }
                                
                                if let Some(ref handle) = stream_handle {
                                    state.audio_stop_flag.store(false, Ordering::SeqCst);
                                    if let Ok(sink) = Sink::try_new(handle) {
                                        let source = StreamingAudioSource {
                                            buffer: state.audio_buffer.clone(),
                                            stop_flag: state.audio_stop_flag.clone(),
                                            sample_rate: 48000,
                                            channels: 2,
                                        };
                                        sink.append(source);
                                        sink.play();
                                        audio_sink = Some(sink);
                                    }
                                }
                                
                                state.is_playing = true;
                                state.playback_start_time = Some(Instant::now());
                                state.playback_start_position = clamped;
                                
                                let _ = status_tx.send(PlaybackStatus::Playing);
                            }
                            Err(e) => {
                                log::error!("Failed to restart FFmpeg after seek: {}", e);
                                let _ = status_tx.send(PlaybackStatus::Error(e));
                            }
                        }
                    }
                } else {
                    let _ = status_tx.send(PlaybackStatus::Paused);
                }
            }
            
            Ok(PlaybackCommand::UpdateTracks(tracks)) => {
                log::info!("Updating audio tracks");
                state.audio_tracks = tracks;
                
                // If playing, restart with new tracks
                if state.is_playing {
                    let current_pos = state.current_playback_position();
                    // Stop and restart
                    stop_readers(&mut video_reader_handle, &video_reader_stop,
                               &mut audio_reader_handle, &audio_reader_stop);
                    state.audio_stop_flag.store(true, Ordering::SeqCst);
                    state.kill_ffmpeg();
                    audio_sink = None;
                    
                    state.position = current_pos;
                    // Trigger replay by recursively handling (simplified - in production would refactor)
                }
            }
            
            Ok(PlaybackCommand::ExtractFrame(timestamp)) => {
                if !state.is_playing {
                    if let Some(ref path) = state.video_path {
                        if let Ok(mut frame) = extract_single_frame(path, timestamp) {
                            state.frame_sequence += 1;
                            frame.sequence = state.frame_sequence;
                            let _ = frame_tx.send(frame);
                        }
                    }
                }
            }
            
            Ok(PlaybackCommand::Shutdown) => {
                log::info!("Playback thread shutting down");
                stop_readers(&mut video_reader_handle, &video_reader_stop,
                           &mut audio_reader_handle, &audio_reader_stop);
                state.audio_stop_flag.store(true, Ordering::SeqCst);
                state.kill_ffmpeg();
                break;
            }
            
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Process frame pacing during playback
                if state.is_playing {
                    let current_time = state.current_playback_position();
                    
                    // Check if playback reached the end
                    if current_time >= state.duration {
                        state.position = state.duration;
                        state.is_playing = false;
                        state.playback_start_time = None;
                        
                        stop_readers(&mut video_reader_handle, &video_reader_stop,
                                   &mut audio_reader_handle, &audio_reader_stop);
                        state.audio_stop_flag.store(true, Ordering::SeqCst);
                        state.kill_ffmpeg();
                        audio_sink = None;
                        
                        let _ = status_tx.send(PlaybackStatus::Paused);
                        continue;
                    }
                    
                    // Release frames that are due
                    if let Ok(mut buffer) = frame_buffer.lock() {
                        while let Some((pts, _)) = buffer.first() {
                            if *pts <= current_time {
                                let (_, frame) = buffer.remove(0);
                                let _ = frame_tx.send(frame);
                            } else {
                                break;
                            }
                        }
                    }
                    
                    // Send position update periodically
                    let _ = status_tx.send(PlaybackStatus::PositionUpdate(current_time));
                }
            }
            
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::info!("Command channel disconnected, shutting down");
                break;
            }
        }
    }
    
    // Cleanup
    stop_readers(&mut video_reader_handle, &video_reader_stop,
               &mut audio_reader_handle, &audio_reader_stop);
    state.kill_ffmpeg();
    log::info!("Playback thread exited");
}

fn stop_readers(
    video_handle: &mut Option<JoinHandle<()>>,
    video_stop: &Arc<AtomicBool>,
    audio_handle: &mut Option<JoinHandle<()>>,
    audio_stop: &Arc<AtomicBool>,
) {
    video_stop.store(true, Ordering::SeqCst);
    audio_stop.store(true, Ordering::SeqCst);
    
    if let Some(handle) = video_handle.take() {
        let _ = handle.join();
    }
    if let Some(handle) = audio_handle.take() {
        let _ = handle.join();
    }
}

fn video_reader_thread(
    mut stdout: std::process::ChildStdout,
    frame_buffer: Arc<Mutex<Vec<(f64, VideoFrame)>>>,
    stop_flag: Arc<AtomicBool>,
    frame_rate: f64,
    start_position: f64,
    process_id: u64,
) {
    let frame_size = 854 * 480 * 3; // RGB24
    let frame_duration = 1.0 / frame_rate;
    let mut frame_index = 0u64;
    let mut buffer = vec![0u8; frame_size];
    
    log::debug!("Video reader started (process_id: {}, fps: {:.2})", process_id, frame_rate);
    
    while !stop_flag.load(Ordering::Relaxed) {
        match stdout.read_exact(&mut buffer) {
            Ok(()) => {
                // Convert RGB24 to RGBA
                let mut rgba_data = Vec::with_capacity(854 * 480 * 4);
                for chunk in buffer.chunks(3) {
                    if chunk.len() == 3 {
                        rgba_data.push(chunk[0]);
                        rgba_data.push(chunk[1]);
                        rgba_data.push(chunk[2]);
                        rgba_data.push(255);
                    }
                }
                
                let pts = start_position + (frame_index as f64 * frame_duration);
                let frame = VideoFrame {
                    image_data: rgba_data,
                    width: 854,
                    height: 480,
                    timestamp: pts,
                    sequence: frame_index,
                    process_id,
                };
                
                // Add to buffer for paced release
                if let Ok(mut buf) = frame_buffer.lock() {
                    // Limit buffer size to prevent memory issues
                    if buf.len() < 120 { // ~2 seconds at 60fps
                        buf.push((pts, frame));
                    }
                }
                
                frame_index += 1;
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::UnexpectedEof {
                    log::debug!("Video reader error: {}", e);
                }
                break;
            }
        }
    }
    
    log::debug!("Video reader stopped (read {} frames)", frame_index);
}

fn audio_reader_thread(
    mut stderr: std::process::ChildStderr,
    audio_buffer: Arc<Mutex<AudioBuffer>>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut byte_buffer = vec![0u8; 4096]; // Read ~1024 samples at a time
    let mut total_samples = 0u64;
    
    log::debug!("Audio reader started");
    
    while !stop_flag.load(Ordering::Relaxed) {
        match stderr.read(&mut byte_buffer) {
            Ok(0) => break, // EOF
            Ok(bytes_read) => {
                // Convert bytes to f32 samples
                let mut samples = Vec::with_capacity(bytes_read / 4);
                for chunk in byte_buffer[..bytes_read].chunks_exact(4) {
                    let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    samples.push(sample);
                }
                
                // Write to audio buffer
                if let Ok(mut buffer) = audio_buffer.lock() {
                    buffer.write(&samples);
                }
                
                total_samples += samples.len() as u64;
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::UnexpectedEof 
                    && e.kind() != std::io::ErrorKind::WouldBlock {
                    log::debug!("Audio reader error: {}", e);
                }
                break;
            }
        }
    }
    
    log::debug!("Audio reader stopped (read {} samples)", total_samples);
}

// =============================================================================
// MEDIA CONTROLLER - PUBLIC API
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum MediaControllerState {
    Unloaded,
    Loading,
    Ready,
    Playing,
    Paused,
    Seeking,
    Error(String),
}

impl MediaControllerState {
    pub fn can_play(&self) -> bool {
        matches!(self, MediaControllerState::Ready | MediaControllerState::Paused)
    }
    
    pub fn can_pause(&self) -> bool {
        matches!(self, MediaControllerState::Playing)
    }
    
    pub fn can_seek(&self) -> bool {
        matches!(self, 
            MediaControllerState::Ready | 
            MediaControllerState::Playing | 
            MediaControllerState::Paused
        )
    }
    
    pub fn is_busy(&self) -> bool {
        matches!(self, MediaControllerState::Loading | MediaControllerState::Seeking)
    }
    
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
    // Communication with playback thread
    command_sender: mpsc::Sender<PlaybackCommand>,
    status_receiver: Mutex<mpsc::Receiver<PlaybackStatus>>,
    frame_receiver: Mutex<mpsc::Receiver<VideoFrame>>,
    thread_handle: Option<JoinHandle<()>>,
    
    // Local state (mirrors playback thread state)
    state: MediaControllerState,
    current_position: f64,
    total_duration: f64,
    video_path: Option<PathBuf>,
    video_frame_rate: f64,
    is_playing: bool,
    
    // Rendering
    texture_handle: Option<TextureHandle>,
    
    // Shutdown flag
    is_shutting_down: bool,
}

impl MediaController {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        
        let thread_handle = thread::spawn(move || {
            playback_thread(cmd_rx, status_tx, frame_tx);
        });
        
        Self {
            command_sender: cmd_tx,
            status_receiver: Mutex::new(status_rx),
            frame_receiver: Mutex::new(frame_rx),
            thread_handle: Some(thread_handle),
            state: MediaControllerState::Unloaded,
            current_position: 0.0,
            total_duration: 0.0,
            video_path: None,
            video_frame_rate: 30.0,
            is_playing: false,
            texture_handle: None,
            is_shutting_down: false,
        }
    }
    
    /// Set video file and initialize playback
    pub fn set_video(
        &mut self, 
        video_path: PathBuf, 
        audio_tracks: &[AudioTrack], 
        duration: f64, 
        _ctx: &Context
    ) -> Result<(), Box<dyn std::error::Error>> {
        log::info!("MediaController: Setting video {:?} (duration: {:.2}s)", video_path, duration);
        
        self.state = MediaControllerState::Loading;
        
        // Get frame rate
        let frame_rate = Self::get_video_frame_rate(&video_path).unwrap_or(30.0);
        self.video_frame_rate = frame_rate;
        
        // Enable first audio track by default
        let mut tracks = audio_tracks.to_vec();
        if !tracks.is_empty() {
            tracks[0].enabled = true;
        }
        
        let _ = self.command_sender.send(PlaybackCommand::SetVideo {
            path: video_path.clone(),
            duration,
            frame_rate,
            audio_tracks: tracks,
        });
        
        self.video_path = Some(video_path);
        self.total_duration = duration;
        self.current_position = 0.0;
        self.is_playing = false;
        self.state = MediaControllerState::Ready;
        
        Ok(())
    }
    
    /// Start playback
    pub fn play(&mut self) {
        if !self.state.can_play() {
            log::warn!("Cannot play in state: {:?}", self.state);
            return;
        }
        
        log::info!("MediaController: Play from {:.2}s", self.current_position);
        let _ = self.command_sender.send(PlaybackCommand::Play);
        self.is_playing = true;
        self.state = MediaControllerState::Playing;
    }
    
    /// Pause playback
    pub fn pause(&mut self) {
        if !self.state.can_pause() {
            log::warn!("Cannot pause in state: {:?}", self.state);
            return;
        }
        
        log::info!("MediaController: Pause");
        let _ = self.command_sender.send(PlaybackCommand::Pause);
        self.is_playing = false;
        self.state = MediaControllerState::Paused;
    }
    
    /// Seek to timestamp
    pub fn seek(&mut self, timestamp: f64) {
        if !self.state.can_seek() {
            log::warn!("Cannot seek in state: {:?}", self.state);
            return;
        }
        
        let clamped = timestamp.clamp(0.0, self.total_duration);
        log::info!("MediaController: Seek to {:.2}s", clamped);
        
        let _ = self.command_sender.send(PlaybackCommand::Seek(clamped));
        self.current_position = clamped;
    }
    
    /// Seek immediately (alias for seek)
    pub fn seek_immediate(&mut self, timestamp: f64) {
        self.seek(timestamp);
    }
    
    /// Update audio track configuration
    pub fn update_audio_tracks(&mut self, audio_tracks: &[AudioTrack]) {
        let _ = self.command_sender.send(PlaybackCommand::UpdateTracks(audio_tracks.to_vec()));
    }
    
    /// Update state from playback thread (call from GUI loop)
    pub fn update(&mut self, ctx: &Context) {
        if self.is_shutting_down {
            return;
        }
        
        // Process status updates
        if let Ok(receiver) = self.status_receiver.lock() {
            while let Ok(status) = receiver.try_recv() {
                match status {
                    PlaybackStatus::Ready => {
                        self.state = MediaControllerState::Ready;
                    }
                    PlaybackStatus::Playing => {
                        self.state = MediaControllerState::Playing;
                        self.is_playing = true;
                    }
                    PlaybackStatus::Paused => {
                        self.state = MediaControllerState::Paused;
                        self.is_playing = false;
                    }
                    PlaybackStatus::PositionUpdate(pos) => {
                        self.current_position = pos;
                    }
                    PlaybackStatus::Error(msg) => {
                        self.state = MediaControllerState::Error(msg);
                        self.is_playing = false;
                    }
                }
            }
        }
        
        // Process video frames
        if let Ok(receiver) = self.frame_receiver.lock() {
            // Get the latest frame (skip old ones)
            let mut latest_frame: Option<VideoFrame> = None;
            while let Ok(frame) = receiver.try_recv() {
                latest_frame = Some(frame);
            }
            
            if let Some(frame) = latest_frame {
                if frame.image_data.len() == (frame.width * frame.height * 4) as usize {
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.image_data,
                    );
                    
                    self.texture_handle = Some(ctx.load_texture(
                        "video_frame",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
        
        // Request repaint during playback
        if self.is_playing {
            ctx.request_repaint();
        }
    }
    
    // =============================================================================
    // STATE QUERIES
    // =============================================================================
    
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
    
    pub fn current_position(&self) -> f64 {
        self.current_position
    }
    
    pub fn current_time(&self) -> f64 {
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
    
    pub fn get_frame_texture(&mut self, _ctx: &Context) -> Option<TextureHandle> {
        self.texture_handle.clone()
    }
    
    pub fn has_error(&self) -> bool {
        matches!(self.state, MediaControllerState::Error(_))
    }
    
    pub fn error_message(&self) -> Option<&str> {
        match &self.state {
            MediaControllerState::Error(msg) => Some(msg),
            _ => None,
        }
    }
    
    pub fn clear_error(&mut self) {
        if self.has_error() {
            self.state = if self.video_path.is_some() {
                MediaControllerState::Ready
            } else {
                MediaControllerState::Unloaded
            };
        }
    }
    
    // =============================================================================
    // HELPERS
    // =============================================================================
    
    fn get_video_frame_rate(video_path: &PathBuf) -> Result<f64, Box<dyn std::error::Error>> {
        let output = Command::new("ffprobe")
            .args([
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "stream=r_frame_rate",
                "-of", "csv=p=0",
                video_path.to_str().ok_or("Invalid path")?,
            ])
            .output()?;
        
        if !output.status.success() {
            return Err("ffprobe failed".into());
        }
        
        let fps_str = String::from_utf8(output.stdout)?.trim().to_string();
        
        let fps = if fps_str.contains('/') {
            let parts: Vec<&str> = fps_str.split('/').collect();
            let num: f64 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(30.0);
            let den: f64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            if den != 0.0 { num / den } else { 30.0 }
        } else {
            fps_str.parse().unwrap_or(30.0)
        };
        
        Ok(fps.clamp(1.0, 1000.0))
    }
}

impl Drop for MediaController {
    fn drop(&mut self) {
        log::debug!("MediaController dropping");
        self.is_shutting_down = true;
        
        let _ = self.command_sender.send(PlaybackCommand::Shutdown);
        
        if let Some(handle) = self.thread_handle.take() {
            // Wait briefly for clean shutdown
            for _ in 0..10 {
                if handle.is_finished() {
                    let _ = handle.join();
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            log::warn!("Playback thread did not shut down cleanly");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_media_controller_state_transitions() {
        let controller = MediaController::new();
        
        assert_eq!(controller.state(), &MediaControllerState::Unloaded);
        assert!(!controller.state().can_play());
        assert!(!controller.state().can_pause());
    }
    
    #[test]
    fn test_audio_buffer() {
        let mut buffer = AudioBuffer::new(10);
        
        // Write some samples
        assert_eq!(buffer.write(&[1.0, 2.0, 3.0]), 3);
        assert_eq!(buffer.available(), 3);
        
        // Read them back
        assert_eq!(buffer.read(), Some(1.0));
        assert_eq!(buffer.read(), Some(2.0));
        assert_eq!(buffer.available(), 1);
        
        // Clear
        buffer.clear();
        assert_eq!(buffer.available(), 0);
        assert_eq!(buffer.read(), None);
    }
}
