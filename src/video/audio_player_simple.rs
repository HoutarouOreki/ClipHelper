// =============================================================================
// SIMPLIFIED SYNCHRONIZED AUDIO PLAYBACK SYSTEM
// =============================================================================
//
// This module provides audio playback synchronized with the embedded video player.
// SIMPLIFIED VERSION to fix performance and stability issues.
//
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use rodio::{OutputStream, Sink};
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

        // Start simplified audio thread - no complex FFmpeg processing yet
        self.start_simple_audio_thread(video_path, track_states);
    }

    fn start_simple_audio_thread(&mut self, _video_path: PathBuf, _audio_tracks: Vec<AudioTrackState>) {
        let (command_sender, command_receiver) = mpsc::channel();
        self.command_sender = Some(command_sender);

        // SIMPLIFIED: Just create a basic audio sink without complex processing
        let audio_thread = thread::spawn(move || {
            let mut current_sink: Option<Arc<Mutex<Sink>>> = None;
            let mut _current_stream: Option<OutputStream> = None;
            
            // Simple command processing loop with timeout to prevent hanging
            loop {
                match command_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(AudioCommand::Play(_start_time)) => {
                        log::debug!("Audio: Play command received (simplified mode)");
                        // For now, just create a silent sink to avoid crashes
                        if let Ok((stream, stream_handle)) = OutputStream::try_default() {
                            if let Ok(sink) = Sink::try_new(&stream_handle) {
                                current_sink = Some(Arc::new(Mutex::new(sink)));
                                _current_stream = Some(stream);
                            }
                        }
                    }
                    Ok(AudioCommand::Pause) => {
                        log::debug!("Audio: Pause command received");
                        if let Some(ref sink) = current_sink {
                            if let Ok(sink_guard) = sink.lock() {
                                sink_guard.pause();
                            }
                        }
                    }
                    Ok(AudioCommand::Seek(_timestamp)) => {
                        log::debug!("Audio: Seek command received");
                        // For now, just acknowledge the seek
                    }
                    Ok(AudioCommand::Stop) => {
                        log::debug!("Audio: Stop command received");
                        current_sink = None;
                        _current_stream = None;
                        break;
                    }
                    Ok(AudioCommand::UpdateTracks(_new_tracks)) => {
                        log::debug!("Audio: Update tracks command received");
                        // For now, just acknowledge the update
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Continue the loop - this prevents infinite blocking
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        log::debug!("Audio: Command receiver disconnected");
                        break;
                    }
                }
            }
            
            log::debug!("Audio: Thread exiting cleanly");
        });

        self.audio_thread = Some(audio_thread);
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
        
        // Wait for thread to finish - with timeout to prevent hanging
        if let Some(thread) = self.audio_thread.take() {
            // Give the thread 1 second to finish cleanly
            let start_time = Instant::now();
            while !thread.is_finished() && start_time.elapsed().as_secs() < 1 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            
            // If thread hasn't finished, try to join it
            if let Err(e) = thread.join() {
                log::warn!("Audio thread didn't join cleanly: {:?}", e);
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
