use std::path::Path;
use std::process::Command;

pub struct WaveformData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub duration: f64,
}

impl WaveformData {
    pub fn generate(audio_file: &Path, track_index: usize) -> anyhow::Result<Self> {
        // Extract audio to temporary WAV file for processing
        let temp_path = std::env::temp_dir().join("temp_audio.wav");
        
        let output = Command::new("ffmpeg")
            .arg("-i").arg(audio_file)
            .arg("-map").arg(format!("0:a:{}", track_index))
            .arg("-acodec").arg("pcm_s16le")
            .arg("-ar").arg("44100")
            .arg("-ac").arg("1") // Mono for waveform
            .arg("-y")
            .arg(&temp_path)
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to extract audio for waveform"));
        }

        // Read WAV file
        let mut reader = hound::WavReader::open(&temp_path)?;
        let spec = reader.spec();
        
        let samples: Result<Vec<f32>, _> = reader
            .samples::<i16>()
            .map(|s| s.map(|sample| sample as f32 / i16::MAX as f32))
            .collect();

        let samples = samples?;
        let duration = samples.len() as f64 / spec.sample_rate as f64;

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);

        Ok(WaveformData {
            samples,
            sample_rate: spec.sample_rate,
            duration,
        })
    }

    pub fn get_peak_at_time(&self, time: f64, window_size: f64) -> f32 {
        let start_sample = ((time - window_size / 2.0) * self.sample_rate as f64) as usize;
        let end_sample = ((time + window_size / 2.0) * self.sample_rate as f64) as usize;
        
        let start_sample = start_sample.min(self.samples.len());
        let end_sample = end_sample.min(self.samples.len());
        
        if start_sample >= end_sample {
            return 0.0;
        }
        
        self.samples[start_sample..end_sample]
            .iter()
            .map(|&s| s.abs())
            .fold(0.0f32, |acc, s| acc.max(s))
    }

    pub fn downsample_for_display(&self, target_width: usize) -> Vec<f32> {
        if self.samples.is_empty() {
            return vec![0.0; target_width];
        }

        let samples_per_pixel = self.samples.len() / target_width;
        let mut result = Vec::with_capacity(target_width);

        for i in 0..target_width {
            let start = i * samples_per_pixel;
            let end = ((i + 1) * samples_per_pixel).min(self.samples.len());
            
            if start < end {
                let peak = self.samples[start..end]
                    .iter()
                    .map(|&s| s.abs())
                    .fold(0.0f32, |acc, s| acc.max(s));
                result.push(peak);
            } else {
                result.push(0.0);
            }
        }

        result
    }
}
