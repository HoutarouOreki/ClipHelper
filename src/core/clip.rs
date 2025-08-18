use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub original_file: PathBuf,
    pub timestamp: DateTime<Utc>,
    pub duration_seconds: u32,
    pub name: Option<String>,
    pub trim_start: f64, // seconds from start
    pub trim_end: f64,   // seconds from start
    pub audio_tracks: Vec<AudioTrack>,
    pub is_deleted: bool,
    pub is_trimmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrack {
    pub index: usize,
    pub enabled: bool,
    pub surround_mode: bool, // true = surround left/right, false = normal
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum ClipDuration {
    Seconds15 = 15,
    Seconds30 = 30,
    Minutes1 = 60,
    Minutes2 = 120,
    Minutes5 = 300,
}

impl Clip {
    pub fn new(file: PathBuf, duration: ClipDuration) -> anyhow::Result<Self> {
        let timestamp = Self::extract_timestamp_from_filename(&file)?;
        let duration_seconds = duration as u32;
        
        Ok(Clip {
            id: uuid::Uuid::new_v4().to_string(),
            original_file: file,
            timestamp,
            duration_seconds,
            name: None,
            trim_start: 0.0,
            trim_end: duration_seconds as f64,
            audio_tracks: Vec::new(),
            is_deleted: false,
            is_trimmed: false,
        })
    }

    pub fn extract_timestamp_from_filename(file: &PathBuf) -> anyhow::Result<DateTime<Utc>> {
        let filename = file.file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid filename"))?;
        
        // Parse "Replay 2025-08-17 21-52-01" format
        if let Some(date_part) = filename.strip_prefix("Replay ") {
            let datetime_str = date_part.replace('-', ":");
            let dt = chrono::NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")?;
            Ok(DateTime::from_naive_utc_and_offset(dt, Utc))
        } else {
            Err(anyhow::anyhow!("Filename doesn't match expected format"))
        }
    }

    pub fn get_output_filename(&self) -> String {
        let original_name = self.original_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("clip");
        
        match &self.name {
            Some(custom_name) => format!("{} - {}", original_name, custom_name),
            None => original_name.to_string(),
        }
    }

    pub fn matches_timestamp(&self, target_time: DateTime<Utc>) -> bool {
        let time_diff = (target_time - self.timestamp).num_seconds().abs();
        time_diff <= 10 // Within 10 seconds
    }
}
