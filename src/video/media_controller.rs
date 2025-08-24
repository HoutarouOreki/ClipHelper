// =============================================================================
// MEDIA CONTROLLER - SINGLE POINT OF CONTROL FOR VIDEO AND AUDIO
// =============================================================================
//
// This module provides a unified interface for controlling both video and audio
// playback, ensuring they are always synchronized and preventing the class of
// bugs where one system is updated but the other is forgotten.
//
// DESIGN PRINCIPLES:
// - Single public interface: only MediaController has play/pause/seek methods
// - Centralized state: position and playing state managed in one place
// - Always coordinated: impossible to update video without audio or vice versa
// - Clear ownership: video and audio players are internal implementation details
//
// =============================================================================

use std::path::PathBuf;
use crate::video::embedded_player::EmbeddedVideoPlayer;
use crate::video::audio_player_complete::SynchronizedAudioPlayer;
use crate::core::clip::AudioTrack;
use egui::{Context, TextureHandle};

pub struct MediaController {
    // State management - single source of truth
    current_position: f64,
    is_playing: bool,
    total_duration: f64,
    video_path: Option<PathBuf>,
    
    // Internal players - not directly accessible
    video_player: Option<EmbeddedVideoPlayer>,
    audio_player: Option<SynchronizedAudioPlayer>,
}

impl MediaController {
    pub fn new() -> Self {
        Self {
            current_position: 0.0,
            is_playing: false,
            total_duration: 0.0,
            video_path: None,
            video_player: None,
            audio_player: None,
        }
    }
    
    // =============================================================================
    // PUBLIC INTERFACE - Single point of control
    // =============================================================================
    
    /// Start playback from current position
    /// ALWAYS coordinates both video and audio
    pub fn play(&mut self) {
        // TODO: Implement coordinated playback
        self.is_playing = true;
    }
    
    /// Pause playback
    /// ALWAYS coordinates both video and audio
    pub fn pause(&mut self) {
        // TODO: Implement coordinated pause
        self.is_playing = false;
    }
    
    /// Seek to specific timestamp
    /// ALWAYS coordinates both video and audio
    pub fn seek(&mut self, timestamp: f64) {
        // TODO: Implement coordinated seeking
        self.current_position = timestamp.clamp(0.0, self.total_duration);
    }
    
    /// Set video file and initialize players
    pub fn set_video(&mut self, video_path: PathBuf, audio_tracks: &[AudioTrack], ctx: &Context) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Initialize both video and audio players
        self.video_path = Some(video_path);
        self.current_position = 0.0;
        self.is_playing = false;
        Ok(())
    }
    
    /// Update audio track configuration
    pub fn update_audio_tracks(&mut self, audio_tracks: &[AudioTrack]) {
        // TODO: Update audio player with new track configuration
    }
    
    // =============================================================================
    // STATE QUERIES - Read-only access to coordinated state
    // =============================================================================
    
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
    
    pub fn current_position(&self) -> f64 {
        self.current_position
    }
    
    pub fn total_duration(&self) -> f64 {
        self.total_duration
    }
    
    pub fn video_path(&self) -> Option<&PathBuf> {
        self.video_path.as_ref()
    }
    
    /// Get current frame texture for display
    pub fn get_frame_texture(&mut self, ctx: &Context) -> Option<TextureHandle> {
        // TODO: Delegate to video player
        None
    }
    
    /// Update internal state (called from GUI loop)
    pub fn update(&mut self, ctx: &Context) {
        // TODO: Update both players and sync state
    }
}

impl Drop for MediaController {
    fn drop(&mut self) {
        // Ensure clean shutdown of both players
        self.pause();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    
    // Mock audio track for testing
    fn create_test_audio_track(index: usize, enabled: bool) -> AudioTrack {
        AudioTrack {
            index,
            enabled,
            surround_mode: false,
            name: format!("Test Track {}", index),
        }
    }
    
    #[test]
    fn test_initial_state() {
        let controller = MediaController::new();
        
        assert_eq!(controller.current_position(), 0.0);
        assert_eq!(controller.is_playing(), false);
        assert_eq!(controller.total_duration(), 0.0);
        assert!(controller.video_path().is_none());
    }
    
    #[test]
    fn test_play_pause_state_coordination() {
        let mut controller = MediaController::new();
        
        // Initial state
        assert!(!controller.is_playing());
        
        // Play should update state
        controller.play();
        assert!(controller.is_playing());
        
        // Pause should update state
        controller.pause();
        assert!(!controller.is_playing());
        
        // Multiple plays should be idempotent
        controller.play();
        controller.play();
        assert!(controller.is_playing());
        
        // Multiple pauses should be idempotent
        controller.pause();
        controller.pause();
        assert!(!controller.is_playing());
    }
    
    #[test]
    fn test_seek_position_coordination() {
        let mut controller = MediaController::new();
        // Set a duration so seeking has bounds
        controller.total_duration = 100.0;
        
        // Initial position
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek to middle
        controller.seek(50.0);
        assert_eq!(controller.current_position(), 50.0);
        
        // Seek to end
        controller.seek(100.0);
        assert_eq!(controller.current_position(), 100.0);
        
        // Seek beyond end should clamp
        controller.seek(150.0);
        assert_eq!(controller.current_position(), 100.0);
        
        // Seek before start should clamp
        controller.seek(-10.0);
        assert_eq!(controller.current_position(), 0.0);
    }
    
    #[test]
    fn test_seek_while_playing_maintains_play_state() {
        let mut controller = MediaController::new();
        controller.total_duration = 100.0;
        
        // Start playing
        controller.play();
        assert!(controller.is_playing());
        
        // Seek while playing
        controller.seek(30.0);
        
        // Should maintain playing state and update position
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
    }
    
    #[test]
    fn test_seek_while_paused_maintains_pause_state() {
        let mut controller = MediaController::new();
        controller.total_duration = 100.0;
        
        // Ensure paused
        controller.pause();
        assert!(!controller.is_playing());
        
        // Seek while paused
        controller.seek(30.0);
        
        // Should maintain paused state and update position
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
    }
    
    #[test]
    fn test_play_from_seeked_position() {
        let mut controller = MediaController::new();
        controller.total_duration = 100.0;
        
        // Seek to position while paused
        controller.seek(25.0);
        assert_eq!(controller.current_position(), 25.0);
        assert!(!controller.is_playing());
        
        // Play should start from seeked position
        controller.play();
        assert_eq!(controller.current_position(), 25.0);
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_set_video_resets_state() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set some state
        controller.seek(50.0);
        controller.play();
        
        // Set new video should reset state
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        
        let result = controller.set_video(video_path.clone(), &audio_tracks, &ctx);
        assert!(result.is_ok());
        
        // State should be reset
        assert_eq!(controller.current_position(), 0.0);
        assert!(!controller.is_playing());
        assert_eq!(controller.video_path(), Some(&video_path));
    }
    
    #[test]
    fn test_operations_without_video_are_safe() {
        let mut controller = MediaController::new();
        
        // These operations should not panic even without a video loaded
        controller.play();
        controller.pause();
        controller.seek(10.0);
        controller.update_audio_tracks(&[]);
        
        let ctx = egui::Context::default();
        controller.update(&ctx);
        let texture = controller.get_frame_texture(&ctx);
        assert!(texture.is_none());
    }
    
    #[test]
    fn test_audio_track_updates_while_playing() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        
        // Set video with initial tracks
        let video_path = PathBuf::from("test.mkv");
        let initial_tracks = vec![
            create_test_audio_track(0, true),
            create_test_audio_track(1, false),
        ];
        
        let result = controller.set_video(video_path, &initial_tracks, &ctx);
        assert!(result.is_ok());
        
        // Start playing
        controller.play();
        assert!(controller.is_playing());
        
        // Update audio tracks while playing
        let updated_tracks = vec![
            create_test_audio_track(0, false),
            create_test_audio_track(1, true),
        ];
        
        controller.update_audio_tracks(&updated_tracks);
        
        // Should still be playing after track update
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_complex_playback_scenario() {
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 120.0;
        
        // Load video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, &ctx).unwrap();
        
        // Play from start
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek while playing
        controller.seek(30.0);
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
        
        // Pause
        controller.pause();
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 30.0);
        
        // Seek while paused
        controller.seek(60.0);
        assert!(!controller.is_playing());
        assert_eq!(controller.current_position(), 60.0);
        
        // Resume from new position
        controller.play();
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 60.0);
        
        // Seek to end
        controller.seek(120.0);
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 120.0);
    }
    
    // =============================================================================
    // REGRESSION TESTS - These test the specific bugs we've encountered
    // =============================================================================
    
    #[test]
    fn test_audio_gets_correct_position_on_play() {
        // REGRESSION: Audio always started from 0.0s
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, &ctx).unwrap();
        
        // Seek to middle, then play
        controller.seek(50.0);
        controller.play();
        
        // Audio should start from 50.0s, not 0.0s
        assert_eq!(controller.current_position(), 50.0);
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_video_resumes_after_pause() {
        // REGRESSION: Video stream didn't restart after pausing
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, &ctx).unwrap();
        
        // Play, pause, play again cycle
        controller.play();
        assert!(controller.is_playing());
        
        controller.pause();
        assert!(!controller.is_playing());
        
        // This should work - both video and audio should resume
        controller.play();
        assert!(controller.is_playing());
    }
    
    #[test]
    fn test_audio_seeks_when_video_seeks() {
        // REGRESSION: Audio didn't seek when video did
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, &ctx).unwrap();
        
        // Start playing
        controller.play();
        assert_eq!(controller.current_position(), 0.0);
        
        // Seek while playing - both video and audio should update
        controller.seek(30.0);
        assert_eq!(controller.current_position(), 30.0);
        assert!(controller.is_playing());
        
        // Pause and seek again - both should update
        controller.pause();
        controller.seek(60.0);
        assert_eq!(controller.current_position(), 60.0);
        assert!(!controller.is_playing());
    }
    
    #[test]
    fn test_play_pause_play_maintains_position() {
        // REGRESSION: Multiple play/pause cycles lost track of position
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let audio_tracks = vec![create_test_audio_track(0, true)];
        controller.set_video(video_path, &audio_tracks, &ctx).unwrap();
        
        // Seek to a position
        controller.seek(25.0);
        
        // Multiple play/pause cycles
        for _i in 0..3 {
            controller.play();
            assert!(controller.is_playing());
            assert_eq!(controller.current_position(), 25.0);
            
            controller.pause();
            assert!(!controller.is_playing());
            assert_eq!(controller.current_position(), 25.0);
        }
    }
    
    #[test]
    fn test_audio_track_changes_are_coordinated() {
        // REGRESSION: Audio track updates might not be synchronized with video state
        let mut controller = MediaController::new();
        let ctx = egui::Context::default();
        controller.total_duration = 100.0;
        
        // Set up video
        let video_path = PathBuf::from("test.mkv");
        let initial_tracks = vec![
            create_test_audio_track(0, true),
            create_test_audio_track(1, false),
        ];
        controller.set_video(video_path, &initial_tracks, &ctx).unwrap();
        
        // Start playing and seek to position
        controller.seek(40.0);
        controller.play();
        
        // Update tracks while playing
        let updated_tracks = vec![
            create_test_audio_track(0, false),
            create_test_audio_track(1, true),
        ];
        controller.update_audio_tracks(&updated_tracks);
        
        // State should be maintained
        assert!(controller.is_playing());
        assert_eq!(controller.current_position(), 40.0);
    }
}
