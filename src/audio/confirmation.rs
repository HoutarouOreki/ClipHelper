use crate::audio::device_manager::{AudioDeviceManager, AudioDeviceInfo};
use crate::core::config::AudioConfirmationConfig;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::fs::File;
use std::io::BufReader;

pub struct AudioConfirmation {
    device_manager: AudioDeviceManager,
    current_output_stream: Option<(OutputStream, OutputStreamHandle)>,
    current_sink: Option<Sink>,
}

impl AudioConfirmation {
    pub fn new() -> anyhow::Result<Self> {
        let device_manager = AudioDeviceManager::new()
            .map_err(|e| {
                log::error!("Failed to initialize audio device manager: {}", e);
                anyhow::anyhow!("Failed to initialize audio device manager: {}", e)
            })?;
        
        Ok(AudioConfirmation {
            device_manager,
            current_output_stream: None,
            current_sink: None,
        })
    }
    
    pub fn get_available_devices(&self) -> &[AudioDeviceInfo] {
        self.device_manager.get_devices()
    }
    
    pub fn refresh_devices(&mut self) -> anyhow::Result<()> {
        self.device_manager.refresh_devices()
            .map_err(|e| {
                log::error!("Failed to refresh audio devices: {}", e);
                anyhow::anyhow!("Failed to refresh audio devices: {}", e)
            })
    }
    
    pub fn play_confirmation_sound(&mut self, config: &AudioConfirmationConfig) -> anyhow::Result<()> {
        if !config.enabled {
            log::debug!("Audio confirmation is disabled, skipping sound playback");
            return Ok(());
        }
        
        let sound_file = match &config.sound_file_path {
            Some(path) => path.clone(),
            None => {
                log::warn!("No sound file configured for audio confirmation");
                return Ok(());
            }
        };
        
        log::debug!("Playing confirmation sound: {}", sound_file.display());
        self.play_sound_file(&sound_file, config)
    }
    
    pub fn stop_current_sound(&mut self) {
        if self.current_sink.is_some() || self.current_output_stream.is_some() {
            log::debug!("Stopping current audio confirmation sound");
            self.current_sink = None;
            self.current_output_stream = None;
        }
    }

    /// Plays a duration-specific confirmation sound based on the clip duration
    pub fn play_duration_confirmation(&mut self, duration: &crate::core::ClipDuration, config: &AudioConfirmationConfig) -> anyhow::Result<()> {
        if !config.enabled {
            log::debug!("Audio confirmation is disabled, skipping duration sound playback");
            return Ok(());
        }

        // Ensure duration sounds exist
        let sounds_dir = crate::audio::generate_duration_confirmation_sounds()
            .map_err(|e| {
                log::error!("Failed to ensure duration confirmation sounds: {}", e);
                anyhow::anyhow!("Failed to ensure duration sounds: {}", e)
            })?;

        let sound_file = match duration {
            crate::core::ClipDuration::Seconds15 => sounds_dir.join("duration_15s.wav"),
            crate::core::ClipDuration::Seconds30 => sounds_dir.join("duration_30s.wav"),
            crate::core::ClipDuration::Minutes1 => sounds_dir.join("duration_1m.wav"),
            crate::core::ClipDuration::Minutes2 => sounds_dir.join("duration_2m.wav"),
            crate::core::ClipDuration::Minutes5 => sounds_dir.join("duration_5m.wav"),
        };

        log::debug!("Playing duration confirmation sound for {:?}: {}", duration, sound_file.display());
        self.play_sound_file(&sound_file, config)
    }

    /// Plays the unmatched clip sound
    pub fn play_unmatched_clip_sound(&mut self, config: &AudioConfirmationConfig) -> anyhow::Result<()> {
        if !config.enabled {
            log::debug!("Audio confirmation is disabled, skipping unmatched sound playback");
            return Ok(());
        }

        // Ensure duration sounds exist (includes unmatched sound)
        let sounds_dir = crate::audio::generate_duration_confirmation_sounds()
            .map_err(|e| {
                log::error!("Failed to ensure duration confirmation sounds: {}", e);
                anyhow::anyhow!("Failed to ensure duration sounds: {}", e)
            })?;

        let sound_file = sounds_dir.join("unmatched_clip.wav");
        log::debug!("Playing unmatched clip sound: {}", sound_file.display());
        self.play_sound_file(&sound_file, config)
    }

    /// Internal method to play a specific sound file
    fn play_sound_file(&mut self, sound_file: &std::path::Path, config: &AudioConfirmationConfig) -> anyhow::Result<()> {
        if !sound_file.exists() {
            log::error!("Sound file does not exist: {}", sound_file.display());
            return Err(anyhow::anyhow!("Sound file not found: {}", sound_file.display()));
        }

        // Get the audio device
        let device = match &config.output_device_name {
            Some(device_name) => {
                match self.device_manager.get_device_by_name(device_name) {
                    Ok(device) => {
                        log::debug!("Using configured audio device: {}", device_name);
                        device
                    }
                    Err(e) => {
                        log::warn!("Failed to get configured device '{}', using default: {}", device_name, e);
                        self.device_manager.get_default_device()
                            .map_err(|e| {
                                log::error!("Failed to get default audio device: {}", e);
                                anyhow::anyhow!("Failed to get default audio device: {}", e)
                            })?
                    }
                }
            }
            None => {
                log::debug!("No device configured, using default audio device");
                self.device_manager.get_default_device()
                    .map_err(|e| {
                        log::error!("Failed to get default audio device: {}", e);
                        anyhow::anyhow!("Failed to get default audio device: {}", e)
                    })?
            }
        };

        // Create output stream for the selected device
        let (_stream, stream_handle) = OutputStream::try_from_device(&device)
            .map_err(|e| {
                log::error!("Failed to create output stream: {}", e);
                anyhow::anyhow!("Failed to create output stream: {}", e)
            })?;

        // Open the sound file
        let file = File::open(sound_file)
            .map_err(|e| {
                log::error!("Failed to open sound file '{}': {}", sound_file.display(), e);
                anyhow::anyhow!("Failed to open sound file: {}", e)
            })?;

        let buf_reader = BufReader::new(file);

        // Decode the audio file
        let source = Decoder::new(buf_reader)
            .map_err(|e| {
                log::error!("Failed to decode sound file '{}': {}", sound_file.display(), e);
                anyhow::anyhow!("Failed to decode sound file: {}", e)
            })?;

        // Create a sink and play the sound
        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| {
                log::error!("Failed to create audio sink: {}", e);
                anyhow::anyhow!("Failed to create audio sink: {}", e)
            })?;

        // Apply volume
        let source_with_volume = source.amplify(config.volume);

        sink.append(source_with_volume);

        // Play the sound (non-blocking)
        sink.play();

        // Store the stream and sink to keep them alive
        self.current_output_stream = Some((_stream, stream_handle));
        self.current_sink = Some(sink);

        log::info!("Successfully started playing sound at volume {:.1}%", config.volume * 100.0);
        Ok(())
    }
}
