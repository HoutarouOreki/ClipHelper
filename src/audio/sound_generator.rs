use hound::{WavWriter, WavSpec, SampleFormat};
use std::path::Path;

/// Generates a simple beep sound for testing audio confirmation
pub fn generate_test_beep(output_path: &Path, frequency: f32, duration_ms: u32) -> anyhow::Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    
    let mut writer = WavWriter::create(output_path, spec)
        .map_err(|e| {
            log::error!("Failed to create WAV writer for test beep: {}", e);
            anyhow::anyhow!("Failed to create WAV writer: {}", e)
        })?;
    
    let samples_per_second = spec.sample_rate as f32;
    let total_samples = (duration_ms as f32 / 1000.0 * samples_per_second) as u32;
    
    for i in 0..total_samples {
        let t = i as f32 / samples_per_second;
        let sample = (t * frequency * 2.0 * std::f32::consts::PI).sin();
        let amplitude = 0.3; // 30% volume to avoid being too loud
        let sample_value = (sample * amplitude * i16::MAX as f32) as i16;
        
        writer.write_sample(sample_value)
            .map_err(|e| {
                log::error!("Failed to write sample to test beep: {}", e);
                anyhow::anyhow!("Failed to write sample: {}", e)
            })?;
    }
    
    writer.finalize()
        .map_err(|e| {
            log::error!("Failed to finalize test beep WAV file: {}", e);
            anyhow::anyhow!("Failed to finalize WAV file: {}", e)
        })?;
    
    log::info!("Generated test beep sound at: {}", output_path.display());
    Ok(())
}

/// Creates a default confirmation sound if none exists
pub fn ensure_default_confirmation_sound() -> anyhow::Result<std::path::PathBuf> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clip-helper");
    
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| {
            log::error!("Failed to create config directory: {}", e);
            anyhow::anyhow!("Failed to create config directory: {}", e)
        })?;
    
    let sound_path = config_dir.join("default_confirmation.wav");
    
    if !sound_path.exists() {
        log::info!("Creating default confirmation sound at: {}", sound_path.display());
        generate_test_beep(&sound_path, 800.0, 200)?; // 800Hz beep for 200ms
    }
    
    Ok(sound_path)
}

/// Generates duration-specific confirmation sounds
pub fn generate_duration_confirmation_sounds() -> anyhow::Result<std::path::PathBuf> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clip-helper");
    
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| {
            log::error!("Failed to create config directory: {}", e);
            anyhow::anyhow!("Failed to create config directory: {}", e)
        })?;
    
    // Generate beep patterns for each duration
    generate_beep_pattern(&config_dir.join("duration_15s.wav"), 1000.0, 1, 100, 50)?; // 1 beep
    generate_beep_pattern(&config_dir.join("duration_30s.wav"), 1000.0, 2, 100, 50)?; // 2 beeps
    generate_beep_pattern(&config_dir.join("duration_1m.wav"), 1000.0, 3, 100, 50)?;  // 3 beeps
    generate_beep_pattern(&config_dir.join("duration_2m.wav"), 1000.0, 4, 100, 50)?;  // 4 beeps
    generate_beep_pattern(&config_dir.join("duration_5m.wav"), 1000.0, 5, 100, 50)?;  // 5 beeps
    
    // Generate low frequency sound for unmatched clips
    generate_test_beep(&config_dir.join("unmatched_clip.wav"), 400.0, 500)?; // 400Hz for 500ms
    
    log::info!("Generated duration confirmation sounds in: {}", config_dir.display());
    Ok(config_dir)
}

/// Generates a pattern of beeps with pauses
fn generate_beep_pattern(output_path: &Path, frequency: f32, beep_count: u32, beep_duration_ms: u32, pause_duration_ms: u32) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    
    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| {
            log::error!("Failed to create WAV writer for beep pattern: {}", e);
            anyhow::anyhow!("Failed to create WAV writer: {}", e)
        })?;
    
    let samples_per_second = spec.sample_rate as f32;
    let beep_samples = (beep_duration_ms as f32 / 1000.0 * samples_per_second) as u32;
    let pause_samples = (pause_duration_ms as f32 / 1000.0 * samples_per_second) as u32;
    
    for beep_index in 0..beep_count {
        // Generate beep
        for i in 0..beep_samples {
            let t = i as f32 / samples_per_second;
            let sample = (t * frequency * 2.0 * std::f32::consts::PI).sin();
            let amplitude = 0.3; // 30% volume
            let sample_value = (sample * amplitude * i16::MAX as f32) as i16;
            
            writer.write_sample(sample_value)
                .map_err(|e| {
                    log::error!("Failed to write beep sample: {}", e);
                    anyhow::anyhow!("Failed to write sample: {}", e)
                })?;
        }
        
        // Add pause between beeps (except after the last beep)
        if beep_index < beep_count - 1 {
            for _ in 0..pause_samples {
                writer.write_sample(0)
                    .map_err(|e| {
                        log::error!("Failed to write pause sample: {}", e);
                        anyhow::anyhow!("Failed to write sample: {}", e)
                    })?;
            }
        }
    }
    
    writer.finalize()
        .map_err(|e| {
            log::error!("Failed to finalize beep pattern WAV file: {}", e);
            anyhow::anyhow!("Failed to finalize WAV file: {}", e)
        })?;
    
    log::debug!("Generated beep pattern with {} beeps at: {}", beep_count, output_path.display());
    Ok(())
}
