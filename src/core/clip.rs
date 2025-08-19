use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub original_file: PathBuf,
    pub timestamp: DateTime<Local>,
    pub duration_seconds: u32, // Target duration (from hotkey)
    pub video_length_seconds: Option<f64>, // Actual video file duration
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
            video_length_seconds: None, // Will be populated later when needed
            name: None,
            trim_start: 0.0,
            trim_end: duration_seconds as f64,
            audio_tracks: Vec::new(),
            is_deleted: false,
            is_trimmed: false,
        })
    }

    pub fn new_without_target(file: PathBuf) -> anyhow::Result<Self> {
        let timestamp = Self::extract_timestamp_from_filename(&file)?;
        
        Ok(Clip {
            id: uuid::Uuid::new_v4().to_string(),
            original_file: file,
            timestamp,
            duration_seconds: 0, // No target duration
            video_length_seconds: None, // Will be populated later when needed
            name: None,
            trim_start: 0.0,
            trim_end: 0.0, // Will be set to video length when loaded
            audio_tracks: Vec::new(),
            is_deleted: false,
            is_trimmed: false,
        })
    }

    pub fn extract_timestamp_from_filename(file: &PathBuf) -> anyhow::Result<DateTime<Local>> {
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
                Ok(Local.from_local_datetime(&dt).unwrap())
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

    pub fn matches_timestamp(&self, target_time: DateTime<Local>) -> bool {
        let time_diff = (target_time - self.timestamp).num_seconds().abs();
        let matches = time_diff <= 10; // Within 10 seconds
        
        log::debug!("Timestamp matching: target={}, clip={}, diff={}s, matches={}", 
            target_time, self.timestamp, time_diff, matches);
        
        matches
    }

    pub fn format_duration(seconds: f64) -> String {
        let total_seconds = seconds as u32;
        let minutes = total_seconds / 60;
        let remaining_seconds = total_seconds % 60;
        
        if minutes > 0 {
            format!("{}m {}s", minutes, remaining_seconds)
        } else {
            format!("{}s", remaining_seconds)
        }
    }

    /// Checks if this clip has a valid target duration set (> 0 seconds)
    pub fn has_target_duration(&self) -> bool {
        self.duration_seconds > 0
    }

    /// Checks if the video file is valid (duration >= 1 second)
    /// Returns false if video info hasn't been loaded yet
    pub fn is_video_valid(&self) -> bool {
        if let Some(length) = self.video_length_seconds {
            length >= 1.0 // Video must be at least 1 second
        } else {
            false // Unknown length = not valid yet
        }
    }

    /// Checks if this clip needs video info to be loaded/updated
    /// Returns true if video info is missing or if the file might still be being written
    pub fn needs_video_info_update(&self) -> bool {
        match self.video_length_seconds {
            None => true, // No video info loaded yet
            Some(length) => length < 1.0, // Invalid length, might still be writing
        }
    }

    /// Sets the target duration for this clip and updates trim points for last X seconds
    /// This is called when a hotkey assigns a specific duration to the clip
    /// The trim will be set to capture the LAST X seconds of the video
    pub fn set_target_duration(&mut self, duration: ClipDuration) {
        self.duration_seconds = duration as u32;
        
        // If we have video length info, set trim to capture last X seconds
        // Otherwise, we'll update the trim when video info becomes available
        if let Some(video_length) = self.video_length_seconds {
            if video_length >= 1.0 {
                let target_seconds = self.duration_seconds as f64;
                // Trim to last X seconds: start = video_length - target, end = video_length
                self.trim_start = (video_length - target_seconds).max(0.0);
                self.trim_end = video_length;
            }
        }
    }

    /// Attempts to populate video information from the file
    /// Returns Ok(true) if video info was successfully loaded and is valid
    /// Returns Ok(false) if file exists but video info is invalid (still being written)
    /// Returns Err if file doesn't exist or other error occurred
    pub fn populate_video_info(&mut self) -> anyhow::Result<bool> {
        use crate::video::VideoProcessor;
        
        // Check if file exists first
        if !self.original_file.exists() {
            return Err(anyhow::anyhow!("File does not exist: {:?}", self.original_file));
        }
        
        match VideoProcessor::get_video_info(&self.original_file) {
            Ok(video_info) => {
                self.video_length_seconds = Some(video_info.duration);
                self.audio_tracks = video_info.audio_tracks;
                
                // Set trim points based on whether we have a target duration
                if self.has_target_duration() {
                    // Trim to last X seconds of the video
                    let target_seconds = self.duration_seconds as f64;
                    self.trim_start = (video_info.duration - target_seconds).max(0.0);
                    self.trim_end = video_info.duration;
                } else if self.trim_end == 0.0 {
                    // No target duration set, use full video length
                    self.trim_start = 0.0;
                    self.trim_end = video_info.duration;
                }
                
                // Return whether the video is valid (duration >= 1 second)
                Ok(video_info.duration >= 1.0)
            }
            Err(e) => {
                // If we can't get video info, the file might still be being written
                // Set duration to 0 to indicate it's invalid but keep trying
                self.video_length_seconds = Some(0.0);
                log::debug!("Video info not available yet for {}: {}", 
                    self.get_output_filename(), e);
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
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
    fn test_clip_without_target_duration() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let clip = Clip::new_without_target(file_path.clone());
        
        assert!(clip.is_ok());
        let clip = clip.unwrap();
        assert_eq!(clip.duration_seconds, 0);
        assert!(!clip.has_target_duration());
        assert_eq!(clip.trim_start, 0.0);
        assert_eq!(clip.trim_end, 0.0);
        assert_eq!(clip.original_file, file_path);
    }

    #[test]
    fn test_set_target_duration() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new_without_target(file_path).unwrap();
        
        // Initially no target duration
        assert!(!clip.has_target_duration());
        
        // Set target duration without video info - trim points won't be set yet
        clip.set_target_duration(ClipDuration::Seconds15);
        assert!(clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 15);
        assert_eq!(clip.trim_end, 0.0); // No video info yet
        
        // When video info becomes available, trim points should be set for last X seconds
        clip.video_length_seconds = Some(60.0);
        clip.set_target_duration(ClipDuration::Seconds15); // Re-apply to set trim points
        assert_eq!(clip.trim_start, 45.0); // 60 - 15 = 45
        assert_eq!(clip.trim_end, 60.0);
    }

    #[test]
    fn test_video_validity() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new_without_target(file_path).unwrap();
        
        // Initially not valid (no video info)
        assert!(!clip.is_video_valid());
        
        // Set valid video length
        clip.video_length_seconds = Some(120.0);
        assert!(clip.is_video_valid());
        
        // Set invalid video length
        clip.video_length_seconds = Some(0.5);
        assert!(!clip.is_video_valid());
    }

    #[test]
    fn test_needs_video_info_update() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new_without_target(file_path).unwrap();
        
        // Initially needs update (no video info)
        assert!(clip.needs_video_info_update());
        
        // Set valid video length
        clip.video_length_seconds = Some(120.0);
        assert!(!clip.needs_video_info_update());
        
        // Set invalid video length (still being written)
        clip.video_length_seconds = Some(0.5);
        assert!(clip.needs_video_info_update());
        
        // Reset to None
        clip.video_length_seconds = None;
        assert!(clip.needs_video_info_update());
    }

    #[test]
    fn test_set_target_duration_fixes_trim_end() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new_without_target(file_path).unwrap();
        
        // Initially no target duration and trim_end is 0
        assert!(!clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 0);
        assert_eq!(clip.trim_end, 0.0);
        
        // Set target duration without video info should just set duration
        clip.set_target_duration(ClipDuration::Seconds30);
        assert!(clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 30);
        // Without video info, trim points won't be set yet
        assert_eq!(clip.trim_end, 0.0);
        
        // When video info becomes available, should trim last X seconds
        clip.video_length_seconds = Some(120.0);
        clip.set_target_duration(ClipDuration::Seconds30); // Re-apply target to set trim points
        assert_eq!(clip.trim_start, 90.0); // 120 - 30 = 90
        assert_eq!(clip.trim_end, 120.0);
        
        // Setting a different target duration should update trim points for last X seconds
        clip.set_target_duration(ClipDuration::Minutes1);
        assert!(clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 60);
        assert_eq!(clip.trim_start, 60.0); // 120 - 60 = 60
        assert_eq!(clip.trim_end, 120.0);
    }

    #[test]
    fn test_target_duration_with_video_info_lifecycle() {
        let file_path = PathBuf::from("Replay 2025-08-17 21-52-01.mkv");
        let mut clip = Clip::new_without_target(file_path).unwrap();
        
        // Initially no target duration and no video info
        assert!(!clip.has_target_duration());
        assert!(clip.needs_video_info_update());
        
        // Set target duration before video info is loaded - trim points won't be set yet
        clip.set_target_duration(ClipDuration::Seconds30);
        assert!(clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 30);
        assert_eq!(clip.trim_end, 0.0); // No video info yet
        
        // Simulate video info being loaded later
        clip.video_length_seconds = Some(60.0);
        assert!(!clip.needs_video_info_update());
        assert!(clip.is_video_valid());
        
        // Re-apply target duration to set trim points for last X seconds
        clip.set_target_duration(ClipDuration::Seconds30);
        assert_eq!(clip.trim_start, 30.0); // 60 - 30 = 30
        assert_eq!(clip.trim_end, 60.0);
        
        // Target duration should still be preserved
        assert!(clip.has_target_duration());
        assert_eq!(clip.duration_seconds, 30);
    }

    #[test]
    fn test_invalid_filename_format() {
        let invalid_path = PathBuf::from("not_a_replay_file.mkv");
        let result = Clip::extract_timestamp_from_filename(&invalid_path);
        
        assert!(result.is_err());
    }
}
