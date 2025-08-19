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

#[derive(Debug, Clone, PartialEq)]
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
            // Split into date and time parts
            let parts: Vec<&str> = date_part.split(' ').collect();
            if parts.len() == 2 {
                let date_part = parts[0]; // "2025-08-17"
                let time_part = parts[1].replace('-', ":"); // "21-52-01" -> "21:52:01"
                let datetime_str = format!("{} {}", date_part, time_part);
                let dt = chrono::NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")?;
                Ok(DateTime::from_naive_utc_and_offset(dt, Utc))
            } else {
                Err(anyhow::anyhow!("Filename doesn't match expected date format"))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc, Datelike, Timelike};
    use std::path::PathBuf;

    #[test]
    fn test_clip_creation() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let clip = Clip::new(file_path.clone(), ClipDuration::Seconds30);
        
        assert!(clip.is_ok());
        let clip = clip.unwrap();
        assert_eq!(clip.duration_seconds, 30);
        assert_eq!(clip.trim_start, 0.0);
        assert_eq!(clip.trim_end, 30.0);
        assert_eq!(clip.original_file, file_path);
    }

    #[test]
    fn test_timestamp_extraction() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let result = Clip::extract_timestamp_from_filename(&file_path);
        
        assert!(result.is_ok());
        let timestamp = result.unwrap();
        
        // Verify the extracted timestamp components
        assert_eq!(timestamp.year(), 2025);
        assert_eq!(timestamp.month(), 8);
        assert_eq!(timestamp.day(), 17);
        assert_eq!(timestamp.hour(), 21);
        assert_eq!(timestamp.minute(), 52);
        assert_eq!(timestamp.second(), 1);
    }

    #[test]
    fn test_timestamp_matching() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let clip = Clip::new(file_path, ClipDuration::Seconds15).unwrap();
        
        // Should match timestamps within 10-second window
        let exact_time = clip.timestamp;
        let within_window = exact_time + chrono::Duration::seconds(5);
        let outside_window = exact_time + chrono::Duration::seconds(15);
        
        assert!(clip.matches_timestamp(exact_time));
        assert!(clip.matches_timestamp(within_window));
        assert!(!clip.matches_timestamp(outside_window));
    }

    #[test]
    fn test_output_filename_generation() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new(file_path, ClipDuration::Seconds15).unwrap();
        
        // Test default naming
        assert_eq!(clip.get_output_filename(), "Replay 2025-08-17 21-52-01");
        
        // Test custom naming
        clip.name = Some("Epic Moment".to_string());
        assert_eq!(clip.get_output_filename(), "Replay 2025-08-17 21-52-01 - Epic Moment");
    }

    #[test]
    fn test_audio_track_configuration() {
        let track = AudioTrack {
            index: 0,
            enabled: true,
            surround_mode: false,
            name: "Desktop Audio".to_string(),
        };
        
        assert_eq!(track.index, 0);
        assert!(track.enabled);
        assert!(!track.surround_mode);
        assert_eq!(track.name, "Desktop Audio");
    }

    #[test]
    fn test_invalid_filename_format() {
        let invalid_path = PathBuf::from("not_a_replay_file.mkv");
        let result = Clip::extract_timestamp_from_filename(&invalid_path);
        
        assert!(result.is_err());
    }
}
