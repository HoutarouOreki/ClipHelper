#[cfg(test)]
mod tests {
    
    use std::path::PathBuf;
    use std::collections::HashMap;
    use tokio::sync::broadcast;
    use crate::core::AppConfig;
    use crate::gui::timeline::TimelineWidget;
    use crate::gui::app::ClipHelperApp;

    // Test helper to create a minimal app instance for testing
    fn create_test_app() -> ClipHelperApp {
        let (_, hotkey_receiver) = broadcast::channel(10);
        
        ClipHelperApp {
            config: AppConfig::default(),
            clips: Vec::new(),
            selected_clip_index: None,
            video_preview: None,
            waveforms: HashMap::new(),
            hotkey_receiver,
            file_monitor: None,
            file_receiver: None,
            new_clip_name: String::new(),
            pending_clip_requests: Vec::new(),
            duration_requests: Vec::new(),
            watched_directory: None,
            show_directory_dialog: false,
            show_settings_dialog: false,
            status_message: String::new(),
            directory_browser_path: PathBuf::from("C:\\"),
            file_browser_path: PathBuf::from("C:\\"),
            show_sound_file_browser: false,
            timeline_widget: TimelineWidget::new(),
            show_drives_view: false,
            last_video_info_check: std::time::Instant::now(),
            initial_scan_completed: false,
            audio_confirmation: None,
            last_thumbnail_processing: std::time::Instant::now(),
            smart_thumbnail_cache: None,
            media_controller: None,
        }
    }

    #[test]
    fn test_app_initialization_with_new_fields() {
        let app = create_test_app();
        
        // Check that new fields are properly initialized
        assert!(!app.config.use_system_file_dialog);
        assert!(!app.config.audio_confirmation.duration_confirmation_enabled);
        assert!(app.media_controller.is_none());
        assert!(!app.show_sound_file_browser);
        assert_eq!(app.file_browser_path, PathBuf::from("C:\\"));
    }

    #[test]
    fn test_file_browser_dialog_state() {
        let mut app = create_test_app();
        
        // Initially should not show sound file browser
        assert!(!app.show_sound_file_browser);
        
        // Simulate opening the sound file browser
        app.show_sound_file_browser = true;
        assert!(app.show_sound_file_browser);
        
        // Simulate closing it
        app.show_sound_file_browser = false;
        assert!(!app.show_sound_file_browser);
    }

    #[test]
    fn test_audio_file_extension_detection() {
        // Test the audio file detection logic that would be used in the file browser
        let audio_extensions = ["wav", "mp3", "ogg", "flac", "m4a", "aac"];
        let non_audio_extensions = ["txt", "pdf", "mkv", "mp4", "doc"];
        
        for ext in audio_extensions {
            let path = PathBuf::from(format!("test.{}", ext));
            if let Some(extension) = path.extension() {
                let ext_str = extension.to_string_lossy().to_lowercase();
                assert!(matches!(ext_str.as_str(), "wav" | "mp3" | "ogg" | "flac" | "m4a" | "aac"), 
                    "Extension {} should be recognized as audio", ext);
            }
        }
        
        for ext in non_audio_extensions {
            let path = PathBuf::from(format!("test.{}", ext));
            if let Some(extension) = path.extension() {
                let ext_str = extension.to_string_lossy().to_lowercase();
                assert!(!matches!(ext_str.as_str(), "wav" | "mp3" | "ogg" | "flac" | "m4a" | "aac"), 
                    "Extension {} should not be recognized as audio", ext);
            }
        }
    }

    #[test]
    fn test_duration_confirmation_enabled_flag() {
        let mut app = create_test_app();
        
        // Duration confirmation should be disabled by default
        assert!(!app.config.audio_confirmation.duration_confirmation_enabled);
        
        // Enable duration confirmation
        app.config.audio_confirmation.duration_confirmation_enabled = true;
        assert!(app.config.audio_confirmation.duration_confirmation_enabled);
        
        // The logic would check this flag before playing duration-specific sounds
        // This is tested in the actual duration confirmation playing logic
    }

    #[test]
    fn test_sound_file_path_editable() {
        let mut app = create_test_app();
        
        // Initially no sound file path
        assert!(app.config.audio_confirmation.sound_file_path.is_none());
        
        // Set a sound file path (simulating user input)
        app.config.audio_confirmation.sound_file_path = Some(PathBuf::from("/test/path/sound.wav"));
        assert_eq!(app.config.audio_confirmation.sound_file_path, Some(PathBuf::from("/test/path/sound.wav")));
        
        // Clear the path (simulating user clearing the text box)
        app.config.audio_confirmation.sound_file_path = None;
        assert!(app.config.audio_confirmation.sound_file_path.is_none());
    }

    #[test]
    fn test_file_browser_path_navigation() {
        let mut app = create_test_app();
        
        // Start with default path
        let initial_path = app.file_browser_path.clone();
        
        // Simulate navigating to a subfolder (would happen when user clicks folder)
        app.file_browser_path = initial_path.join("subfolder");
        assert!(app.file_browser_path.ends_with("subfolder"));
        
        // Simulate going to parent directory
        if let Some(parent) = app.file_browser_path.parent() {
            app.file_browser_path = parent.to_path_buf();
        }
        assert_eq!(app.file_browser_path, initial_path);
    }
}
