#[cfg(test)]
mod tests {
    
    use std::path::PathBuf;
    use crate::core::{AppConfig, AudioConfirmationConfig};

    #[test]
    fn test_audio_confirmation_config_default() {
        let config = AudioConfirmationConfig::default();
        assert!(!config.enabled);
        assert!(config.sound_file_path.is_none());
        assert!(config.output_device_name.is_none());
        assert_eq!(config.volume, 0.5);
        assert!(!config.duration_confirmation_enabled);
        assert!(config.unmatched_sound_enabled); // Default to true
    }

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert!(!config.use_system_file_dialog);
        assert!(!config.audio_confirmation.duration_confirmation_enabled);
        assert!(!config.audio_confirmation.enabled);
        assert!(config.audio_confirmation.unmatched_sound_enabled); // Default to true
    }

    #[test]
    fn test_app_config_serialization() {
        let mut config = AppConfig::default();
        config.use_system_file_dialog = true;
        config.audio_confirmation.duration_confirmation_enabled = true;
        config.audio_confirmation.unmatched_sound_enabled = false;
        config.audio_confirmation.sound_file_path = Some(PathBuf::from("/test/path/sound.wav"));

        let serialized = serde_json::to_string(&config).expect("Failed to serialize config");
        let deserialized: AppConfig = serde_json::from_str(&serialized).expect("Failed to deserialize config");

        assert_eq!(config.use_system_file_dialog, deserialized.use_system_file_dialog);
        assert_eq!(config.audio_confirmation.duration_confirmation_enabled, deserialized.audio_confirmation.duration_confirmation_enabled);
        assert_eq!(config.audio_confirmation.unmatched_sound_enabled, deserialized.audio_confirmation.unmatched_sound_enabled);
        assert_eq!(config.audio_confirmation.sound_file_path, deserialized.audio_confirmation.sound_file_path);
    }

    #[test]
    fn test_audio_confirmation_config_with_duration_enabled() {
        let mut config = AudioConfirmationConfig::default();
        config.duration_confirmation_enabled = true;
        config.enabled = true;
        config.volume = 0.8;
        config.sound_file_path = Some(PathBuf::from("/test/sound.wav"));
        config.unmatched_sound_enabled = false; // Test toggling off

        assert!(config.enabled);
        assert!(config.duration_confirmation_enabled);
        assert!(!config.unmatched_sound_enabled);
        assert_eq!(config.volume, 0.8);
        assert_eq!(config.sound_file_path, Some(PathBuf::from("/test/sound.wav")));
    }

    #[test]
    fn test_config_backward_compatibility() {
        // Test that old config files without new fields can still be loaded
        let old_config_json = r#"{
            "obs_replay_directory": "./replays",
            "output_directory": "./output",
            "deleted_directory": "./output/deleted",
            "trimmed_directory": "./output/trimmed",
            "last_watched_directory": null,
            "ffmpeg_path": null,
            "hotkeys": {},
            "audio_confirmation": {
                "enabled": false,
                "sound_file_path": null,
                "output_device_name": null,
                "volume": 0.5
            }
        }"#;

        let config: AppConfig = serde_json::from_str(old_config_json).expect("Failed to parse old config");
        
        // New fields should have default values
        assert!(!config.use_system_file_dialog);
        assert!(!config.audio_confirmation.duration_confirmation_enabled);
        assert!(config.audio_confirmation.unmatched_sound_enabled); // Default to true
    }

    #[test]
    fn test_file_browser_preference_setting() {
        let mut config = AppConfig::default();
        assert!(!config.use_system_file_dialog); // Default to built-in browser

        config.use_system_file_dialog = true;
        assert!(config.use_system_file_dialog);

        config.use_system_file_dialog = false;
        assert!(!config.use_system_file_dialog);
    }

    #[test]
    fn test_unmatched_sound_setting() {
        let mut config = AudioConfirmationConfig::default();
        assert!(config.unmatched_sound_enabled); // Default to enabled

        config.unmatched_sound_enabled = false;
        assert!(!config.unmatched_sound_enabled);

        config.unmatched_sound_enabled = true;
        assert!(config.unmatched_sound_enabled);
    }
}
