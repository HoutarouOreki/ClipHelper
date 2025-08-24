// =============================================================================
// COMPLETE SYNCHRONIZED AUDIO PLAYBACK SYSTEM
// =============================================================================
//
// This module provides audio playback synchronized with the embedded video player.
// Plays the actual mixed audio that would be rendered into the final video.
//
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use rodio::{OutputStream, Sink, Source};
use crate::core::clip::AudioTrack;

pub struct SynchronizedAudioPlayer {
    audio_sink: Option<Arc<Mutex<Sink>>>,
    _audio_stream: Option<OutputStream>,
    current_position: f64,
    is_playing: bool,
    total_duration: f64,
    last_seek_time: Instant,
    audio_thread: Option<thread::JoinHandle<()>>,
    command_sender: Option<mpsc::Sender<AudioCommand>>,
    video_path: Option<PathBuf>,
    current_tracks: Vec<AudioTrackState>,
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
    Play(f64),
    Pause,
    Seek(f64),
    Stop,
    UpdateTracks(Vec<AudioTrackState>),
}

// Simple audio source that generates mixed audio from FFmpeg output
struct MixedAudioSource {
    sample_rate: u32,
    channels: u16,
    samples: std::vec::IntoIter<f32>,
}

impl Iterator for MixedAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.samples.next()
    }
}

impl Source for MixedAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.samples.len().into()
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
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
            video_path: None,
            current_tracks: Vec::new(),
        })
    }

    pub fn set_video(&mut self, video_path: PathBuf, duration: f64, audio_tracks: &[AudioTrack]) {
        self.stop();
        self.total_duration = duration;
        self.current_position = 0.0;
        self.video_path = Some(video_path.clone());
        
        // Convert audio tracks to our internal format
        self.current_tracks = audio_tracks.iter().map(|track| {
            AudioTrackState {
                index: track.index,
                enabled: track.enabled,
                surround_mode: track.surround_mode,
                name: track.name.clone(),
            }
        }).collect();

        // Start audio processing thread
        self.start_audio_thread(video_path);
    }

    fn start_audio_thread(&mut self, video_path: PathBuf) {
        let (command_sender, command_receiver) = mpsc::channel();
        self.command_sender = Some(command_sender);
        let tracks = self.current_tracks.clone();

        let audio_thread = thread::spawn(move || {
            Self::audio_processing_thread(video_path, tracks, command_receiver);
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
        
        // Create audio output stream once
        let (stream, stream_handle) = match OutputStream::try_default() {
            Ok(output) => output,
            Err(e) => {
                log::error!("Failed to create audio output stream: {}", e);
                return;
            }
        };
        _current_stream = Some(stream);

        loop {
            match command_receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(AudioCommand::Play(start_time)) => {
                    log::debug!("Audio: Starting playback from {:.2}s", start_time);
                    current_position = start_time;
                    
                    // Stop current playback
                    current_sink = None;

                    // Create new sink and start playing mixed audio
                    match Sink::try_new(&stream_handle) {
                        Ok(sink) => {
                            // Generate mixed audio source
                            if let Some(audio_source) = Self::create_mixed_audio_source(&video_path, &audio_tracks, start_time) {
                                sink.append(audio_source);
                                sink.play();
                                current_sink = Some(Arc::new(Mutex::new(sink)));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to create audio sink: {}", e);
                        }
                    }
                }
                Ok(AudioCommand::Pause) => {
                    log::debug!("Audio: Pausing playback");
                    if let Some(ref sink) = current_sink {
                        if let Ok(sink_guard) = sink.lock() {
                            sink_guard.pause();
                        }
                    }
                }
                Ok(AudioCommand::Seek(timestamp)) => {
                    log::debug!("Audio: Seeking to {:.2}s", timestamp);
                    current_position = timestamp;
                    
                    // Stop current audio and restart from new position
                    current_sink = None;
                }
                Ok(AudioCommand::Stop) => {
                    log::debug!("Audio: Stopping playback");
                    current_sink = None;
                    break;
                }
                Ok(AudioCommand::UpdateTracks(new_tracks)) => {
                    log::debug!("Audio: Updating track configuration");
                    audio_tracks = new_tracks;
                    
                    // If currently playing, restart with new track configuration
                    let should_restart = if let Some(ref sink) = current_sink {
                        if let Ok(sink_guard) = sink.lock() {
                            !sink_guard.is_paused()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    
                    if should_restart {
                        current_sink = None;
                        
                        match Sink::try_new(&stream_handle) {
                            Ok(new_sink) => {
                                if let Some(audio_source) = Self::create_mixed_audio_source(&video_path, &audio_tracks, current_position) {
                                    new_sink.append(audio_source);
                                    new_sink.play();
                                    current_sink = Some(Arc::new(Mutex::new(new_sink)));
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to restart audio with new tracks: {}", e);
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Continue loop - prevents hanging
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::debug!("Audio: Command receiver disconnected");
                    break;
                }
            }
        }
        
        log::debug!("Audio: Processing thread exiting cleanly");
    }

    fn create_mixed_audio_source(
        video_path: &PathBuf,
        audio_tracks: &[AudioTrackState],
        start_time: f64
    ) -> Option<MixedAudioSource> {
        // Generate mixed audio using FFmpeg - same logic as VideoProcessor
        let enabled_tracks: Vec<_> = audio_tracks.iter().filter(|t| t.enabled).collect();
        
        if enabled_tracks.is_empty() {
            // No enabled tracks - create silence
            return Some(Self::create_silence_source());
        }
        
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.arg("-ss").arg(start_time.to_string())
            .arg("-i").arg(video_path)
            .arg("-t").arg("5.0"); // Generate 5 seconds at a time for responsiveness
        
        // Build filter complex for mixing
        if enabled_tracks.len() == 1 {
            let track = enabled_tracks[0];
            if track.surround_mode {
                cmd.arg("-filter_complex")
                    .arg(format!("[0:a:{}]channelmap=map=FL|FR[mixed]", track.index))
                    .arg("-map").arg("[mixed]");
            } else {
                cmd.arg("-map").arg(format!("0:a:{}", track.index));
            }
        } else {
            // Multiple tracks - need to mix them
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
                format!("{}amix=inputs={}[mixed]", mix_inputs.join(""), enabled_tracks.len())
            } else {
                format!("{};{}amix=inputs={}[mixed]", 
                    filter_parts.join(";"),
                    mix_inputs.join(""),
                    enabled_tracks.len())
            };
            
            cmd.arg("-filter_complex").arg(&filter_complex)
                .arg("-map").arg("[mixed]");
        }
        
        // Output as raw audio
        cmd.arg("-f").arg("f32le")
            .arg("-ac").arg("2")
            .arg("-ar").arg("48000")
            .arg("-loglevel").arg("error")
            .arg("pipe:1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        
        match cmd.output() {
            Ok(output) if output.status.success() => {
                // Convert raw bytes to f32 samples
                let mut samples = Vec::new();
                let bytes = output.stdout;
                
                for chunk in bytes.chunks_exact(4) {
                    if let Ok(array) = chunk.try_into() {
                        let sample = f32::from_le_bytes(array);
                        samples.push(sample);
                    }
                }
                
                if !samples.is_empty() {
                    Some(MixedAudioSource {
                        sample_rate: 48000,
                        channels: 2,
                        samples: samples.into_iter(),
                    })
                } else {
                    log::warn!("FFmpeg succeeded but produced no audio samples");
                    Some(Self::create_silence_source())
                }
            }
            Ok(output) => {
                log::warn!("FFmpeg audio generation failed with non-zero exit code");
                log::warn!("FFmpeg exit status: {:?}", output.status);
                if !output.stderr.is_empty() {
                    log::warn!("FFmpeg stderr: {}", String::from_utf8_lossy(&output.stderr));
                }
                if !output.stdout.is_empty() {
                    log::warn!("FFmpeg stdout length: {} bytes", output.stdout.len());
                }
                log::warn!("FFmpeg command was: {:?}", cmd);
                Some(Self::create_silence_source())
            }
            Err(e) => {
                log::warn!("Failed to execute FFmpeg command: {}", e);
                log::warn!("FFmpeg command was: {:?}", cmd);
                Some(Self::create_silence_source())
            }
        }
    }

    fn create_silence_source() -> MixedAudioSource {
        // Create 5 seconds of silence
        let samples = vec![0.0f32; 48000 * 2 * 5]; // 48kHz, stereo, 5 seconds
        MixedAudioSource {
            sample_rate: 48000,
            channels: 2,
            samples: samples.into_iter(),
        }
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
        self.current_tracks = audio_tracks.iter().map(|track| {
            AudioTrackState {
                index: track.index,
                enabled: track.enabled,
                surround_mode: track.surround_mode,
                name: track.name.clone(),
            }
        }).collect();

        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::UpdateTracks(self.current_tracks.clone()));
        }
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
        
        if let Some(ref sender) = self.command_sender {
            let _ = sender.send(AudioCommand::Stop);
        }
        
        // Clean thread termination with timeout
        if let Some(thread) = self.audio_thread.take() {
            let start_time = Instant::now();
            while !thread.is_finished() && start_time.elapsed().as_millis() < 500 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            
            if let Err(e) = thread.join() {
                log::warn!("Audio thread join failed: {:?}", e);
            }
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
