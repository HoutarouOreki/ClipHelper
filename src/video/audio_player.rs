// =============================================================================
// SYNCHRONIZED AUDIO PLAYBACK SYSTEM
// =============================================================================
//
// This module provides audio playback synchronized with the embedded video player.
// It plays the mixed audio that would be rendered into the final trimmed video,
// respecting the current audio track enable/disable state and surround mode settings.
//
// KEY FEATURES:
// - Mixed audio playback using the same FFmpeg filter as video rendering
// - Synchronized seeking with video timeline
// - Respects enabled/disabled audio tracks from UI
// - Handles surround sound channel mapping (FL|FR)
// - Non-blocking audio stream management
//
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use rodio::{Decoder, OutputStream, Sink};
use std::io::BufReader;
use crate::core::clip::AudioTrack;

pub struct SynchronizedAudioPlayer {
    audio_sink: Option<Arc<Mutex<Sink>>>,
    _audio_stream: Option<OutputStream>, // Keep alive for the duration
    current_position: f64,
    is_playing: bool,
    total_duration: f64,
    last_seek_time: Instant,
    audio_thread: Option<thread::JoinHandle<()>>,
    command_sender: Option<mpsc::Sender<AudioCommand>>,
}

#[derive(Debug, Clone)]
pub struct AudioTrackState {
    pub index: usize,
    pub enabled: bool,
    pub surround_mode: bool,
    pub name: String,
}

#[derive(Debug)]
enum AudioCommand {
    Play(f64), // Start playing from timestamp
    Pause,
    Seek(f64),
    Stop,
    UpdateTracks(Vec<AudioTrackState>), // Update audio track mixing
}

impl SynchronizedAudioPlayer {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            audio_sink: None,
            _audio_stream: None,
            current_position: 0.0,
            is_playing: false,
            total_duration: 0.0,
            last_seek_time: Instant::now(),
            audio_thread: None,
            command_sender: None,
        })
    }

    pub fn set_video(&mut self, video_path: PathBuf, duration: f64, audio_tracks: &[AudioTrack]) {
        self.stop();
        self.total_duration = duration;
        self.current_position = 0.0;
        
        // Convert audio tracks to our internal format
        let track_states: Vec<AudioTrackState> = audio_tracks.iter().map(|track| {
            AudioTrackState {
                index: track.index,
                enabled: track.enabled,
                surround_mode: track.surround_mode,
                name: track.name.clone(),
            }
        }).collect();

        // Start audio processing thread
        self.start_audio_thread(video_path, track_states);
    }

    fn start_audio_thread(&mut self, video_path: PathBuf, audio_tracks: Vec<AudioTrackState>) {
        let (command_sender, command_receiver) = mpsc::channel();
        self.command_sender = Some(command_sender);

        let audio_thread = thread::spawn(move || {
            Self::audio_processing_thread(video_path, audio_tracks, command_receiver);
        });

        self.audio_thread = Some(audio_thread);
    }

    fn audio_processing_thread(
        video_path: PathBuf,
        mut audio_tracks: Vec<AudioTrackState>,
        command_receiver: mpsc::Receiver<AudioCommand>
    ) {
        let mut current_sink: Option<Arc<Mutex<Sink>>> = None;
        let mut _current_stream: Option<OutputStream> = None;
        let mut current_position = 0.0;

        // Create a timeout for command reception to avoid infinite blocking
        while let Ok(command) = command_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            match command {
                AudioCommand::Play(start_time) => {
                    log::debug!("Audio: Starting playback from {:.2}s", start_time);
                    current_position = start_time;
                    
                    // Stop current playback
                    current_sink = None;
                    _current_stream = None;

                    // Start new audio stream - but don't block the thread with long FFmpeg calls
                    match Self::start_audio_stream(&video_path, &audio_tracks, start_time) {
                        Ok((stream, sink)) => {
                            current_sink = Some(Arc::new(Mutex::new(sink)));
                            _current_stream = Some(stream);
                        }
                        Err(e) => {
                            log::error!("Failed to start audio stream: {}", e);
                        }
                    }
                }
                AudioCommand::Pause => {
                    log::debug!("Audio: Pausing playbook");
                    if let Some(ref sink) = current_sink {
                        if let Ok(sink_guard) = sink.lock() {
                            sink_guard.pause();
                        }
                    }
                }
                AudioCommand::Seek(timestamp) => {
                    log::debug!("Audio: Seeking to {:.2}s", timestamp);
                    current_position = timestamp;
                    
                    // For audio seeking, we need to restart the stream from the new position
                    current_sink = None;
                    _current_stream = None;
                }
                AudioCommand::Stop => {
                    log::debug!("Audio: Stopping playback");
                    current_sink = None;
                    _current_stream = None;
                    break;
                }
                AudioCommand::UpdateTracks(new_tracks) => {
                    log::debug!("Audio: Updating track configuration");
                    audio_tracks = new_tracks;
                    
                    // If currently playing, restart with new track configuration
                    if current_sink.is_some() {
                        current_sink = None;
                        _current_stream = None;

                        // Restart audio with new track mix - lightweight version
                        match Self::start_audio_stream(&video_path, &audio_tracks, current_position) {
                            Ok((stream, sink)) => {
                                current_sink = Some(Arc::new(Mutex::new(sink)));
                                _current_stream = Some(stream);
                            }
                            Err(e) => {
                                log::error!("Failed to restart audio with new tracks: {}", e);
                            }
                        }
                    }
                }
                }
                AudioCommand::Pause => {
                    log::debug!("Audio: Pausing playback");
                    if let Some(ref sink) = current_sink {
                        if let Ok(sink_guard) = sink.lock() {
                            sink_guard.pause();
                        }
                    }
                }
                AudioCommand::Seek(timestamp) => {
                    log::debug!("Audio: Seeking to {:.2}s", timestamp);
                    current_position = timestamp;
                    
                    // For audio seeking, we need to restart the stream from the new position
                    // This is similar to how video seeking works
                    current_sink = None;
                    _current_stream = None;

                    // Only restart if we were playing
                    // The video player will send a Play command if needed
                }
                AudioCommand::Stop => {
                    log::debug!("Audio: Stopping playback");
                    current_sink = None;
                    _current_stream = None;
                }
                AudioCommand::UpdateTracks(new_tracks) => {
                    log::debug!("Audio: Updating track configuration");
                    audio_tracks = new_tracks;
                    
                    // If currently playing, restart with new track configuration
                    if current_sink.is_some() {
                        // Restart audio with new track mix
                        current_sink = None;
                        _current_stream = None;

                        if let Ok((stream, stream_handle)) = OutputStream::try_default() {
                            if let Ok(sink) = Sink::try_new(&stream_handle) {
                                if let Ok(audio_data) = Self::generate_mixed_audio(&video_path, &audio_tracks, current_position) {
                                    if let Ok(source) = Decoder::new(BufReader::new(std::io::Cursor::new(audio_data))) {
                                        sink.append(source);
                                        sink.play();
                                        
                                        current_sink = Some(Arc::new(Mutex::new(sink)));
                                        _current_stream = Some(stream);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn generate_mixed_audio(
        video_path: &PathBuf,
        audio_tracks: &[AudioTrackState],
        start_time: f64
    ) -> anyhow::Result<Vec<u8>> {
        // Use the same FFmpeg filter logic as VideoProcessor::trim_clip
        // but output audio-only stream for playback
        
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-i").arg(video_path);
        cmd.arg("-ss").arg(start_time.to_string());
        
        // Generate audio filter complex - same logic as video processing
        let enabled_tracks: Vec<_> = audio_tracks.iter().filter(|t| t.enabled).collect();
        
        if !enabled_tracks.is_empty() {
            let mut audio_inputs = Vec::new();
            
            for (i, track) in enabled_tracks.iter().enumerate() {
                if track.surround_mode {
                    // Map to surround left/right
                    audio_inputs.push(format!("[0:a:{}]channelmap=map=FL|FR[a{}]", track.index, i));
                } else {
                    audio_inputs.push(format!("[0:a:{}][a{}]", track.index, i));
                }
            }
            
            let filter_complex = if audio_inputs.len() > 1 {
                format!("{}{}amix=inputs={}[mixed]", 
                    audio_inputs.join(";"), 
                    ";",
                    audio_inputs.len()
                )
            } else {
                // Single track, just rename it
                format!("{}[mixed]", audio_inputs[0])
            };
            
            cmd.arg("-filter_complex").arg(&filter_complex);
            cmd.arg("-map").arg("[mixed]");
        } else {
            // No enabled tracks - generate silence
            cmd.arg("-f").arg("lavfi").arg("-i").arg("anullsrc=channel_layout=stereo:sample_rate=48000");
            cmd.arg("-map").arg("1:a");
        }
        
        // Output as WAV to memory
        cmd.arg("-f").arg("wav");
        cmd.arg("-ac").arg("2"); // Stereo output
        cmd.arg("-ar").arg("48000"); // 48kHz sample rate
        cmd.arg("-t").arg("30"); // Limit to 30 seconds for performance
        cmd.arg("pipe:1"); // Output to stdout
        
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        
        let output = cmd.output()?;
        
        if !output.status.success() {
            return Err(anyhow::anyhow!("FFmpeg audio generation failed"));
        }
        
        Ok(output.stdout)
    }

    pub fn play(&mut self) {
        self.is_playing = true;
        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::Play(self.current_position));
        }
    }

    pub fn pause(&mut self) {
        self.is_playing = false;
        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::Pause);
        }
    }

    pub fn seek(&mut self, timestamp: f64) {
        self.current_position = timestamp.clamp(0.0, self.total_duration);
        self.last_seek_time = Instant::now();
        
        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::Seek(self.current_position));
        }
    }

    pub fn update_audio_tracks(&mut self, audio_tracks: &[AudioTrack]) {
        let track_states: Vec<AudioTrackState> = audio_tracks.iter().map(|track| {
            AudioTrackState {
                index: track.index,
                enabled: track.enabled,
                surround_mode: track.surround_mode,
                name: track.name.clone(),
            }
        }).collect();

        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::UpdateTracks(track_states));
        }
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
        
        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::Stop);
        }
        
        // Wait for thread to finish
        if let Some(thread) = self.audio_thread.take() {
            let _ = thread.join();
        }
        
        self.command_sender = None;
        self.audio_sink = None;
        self._audio_stream = None;
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn get_position(&self) -> f64 {
        self.current_position
    }
}

impl Drop for SynchronizedAudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}
