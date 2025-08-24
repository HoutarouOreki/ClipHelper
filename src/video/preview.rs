use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::time::Instant;

pub struct VideoPreview {
    pub current_time: f64,
    pub is_playing: bool,
    pub total_duration: f64,
    pub video_path: Option<PathBuf>,
    pub current_thumbnail_cache_key: Option<String>,
    embedded_process: Option<Child>,
    smart_thumbnail_cache: Option<Arc<crate::video::SmartThumbnailCache>>,
    last_thumbnail_request: Option<Instant>,
}

impl VideoPreview {
    pub fn new(duration: f64) -> Self {
        Self {
            current_time: 0.0,
            is_playing: false,
            total_duration: duration,
            video_path: None,
            current_thumbnail_cache_key: None,
            embedded_process: None,
            smart_thumbnail_cache: None,
            last_thumbnail_request: None,
        }
    }

    pub fn set_video(&mut self, video_path: PathBuf, duration: f64) {
        self.stop(); // Stop any current playback
        self.video_path = Some(video_path);
        self.total_duration = duration;
        self.current_time = 0.0;
        self.request_thumbnail_for_current_time();
    }

    pub fn set_smart_thumbnail_cache(&mut self, cache: Arc<crate::video::SmartThumbnailCache>) {
        self.smart_thumbnail_cache = Some(cache);
        self.request_thumbnail_for_current_time();
    }

    fn request_thumbnail_for_current_time(&mut self) {
        self.request_thumbnail_for_current_time_with_cooldown(true);
    }

    fn request_thumbnail_for_current_time_with_cooldown(&mut self, use_cooldown: bool) {
        // Reduced cooldown for more responsive clicking (was 50ms)
        let now = Instant::now();
        if use_cooldown {
            if let Some(last_request) = self.last_thumbnail_request {
                if now.duration_since(last_request).as_millis() < 20 {
                    return; // Very brief cooldown to prevent extreme spam
                }
            }
        }

        if let (Some(path), Some(cache)) = (&self.video_path, &self.smart_thumbnail_cache) {
            // Request thumbnail for current position
            cache.request_thumbnail(path, self.current_time);
            
            // Only do predictive caching if we haven't requested recently
            if self.last_thumbnail_request.is_none() || 
               now.duration_since(self.last_thumbnail_request.unwrap()).as_millis() > 200 {
                cache.precache_around_timestamp(path, self.current_time, self.total_duration);
            }
            
            self.last_thumbnail_request = Some(now);
        }
    }

    /// Request thumbnail immediately without cooldown (for user clicks)
    pub fn request_thumbnail_immediate(&mut self) {
        self.request_thumbnail_for_current_time_with_cooldown(false);
    }

    pub fn seek_to(&mut self, time: f64) {
        let new_time = time.clamp(0.0, self.total_duration);
        // More responsive seeking - always request thumbnail for user clicks
        // But still avoid tiny movements for programmatic updates
        if (self.current_time - new_time).abs() > 0.01 {
            self.current_time = new_time;
            self.request_thumbnail_for_current_time();
            
            // If playing, restart playback at new position
            if self.is_playing {
                self.stop();
                self.play();
            }
        }
    }

    /// Update position without restarting playback (for sync purposes)
    pub fn sync_position(&mut self, time: f64) {
        let new_time = time.clamp(0.0, self.total_duration);
        self.current_time = new_time;
        // Don't request thumbnail or restart playback for sync updates
    }

    pub fn skip_forward(&mut self, seconds: f64) {
        self.seek_to(self.current_time + seconds);
    }

    pub fn skip_backward(&mut self, seconds: f64) {
        self.seek_to(self.current_time - seconds);
    }

    pub fn play(&mut self) {
        // For embedded playback, we'll request continuous thumbnails during playback
        // rather than using external processes
        if !self.is_playing {
            self.is_playing = true;
            self.request_thumbnail_for_current_time();
            log::info!("Started embedded video playback at {:.3}s", self.current_time);
        }
    }

    pub fn pause(&mut self) {
        self.stop();
        self.is_playing = false;
    }

    pub fn stop(&mut self) {
        if let Some(mut process) = self.embedded_process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
        self.is_playing = false;
    }

    pub fn toggle_playback(&mut self) {
        if self.is_playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn goto_start(&mut self) {
        self.seek_to(0.0);
    }

    pub fn goto_last_5_seconds(&mut self) {
        self.seek_to((self.total_duration - 5.0).max(0.0));
    }

    pub fn get_current_thumbnail(&self) -> Option<crate::video::CachedThumbnail> {
        if let (Some(path), Some(cache)) = (&self.video_path, &self.smart_thumbnail_cache) {
            cache.get_cached_thumbnail(path, self.current_time)
        } else {
            None
        }
    }

    /// Update current time for UI synchronization (called periodically)
    pub fn update_time(&mut self, delta_time: f32) {
        if self.is_playing {
            let old_time = self.current_time;
            self.current_time += delta_time as f64;
            if self.current_time >= self.total_duration {
                self.stop();
                self.current_time = self.total_duration;
            }
            
            // Request new thumbnail more frequently during playback for smooth video
            if (self.current_time - old_time).abs() > 0.5 {
                self.request_thumbnail_for_current_time();
            }
        }
    }

    /// Check if FFplay process is still running
    pub fn is_process_alive(&mut self) -> bool {
        if let Some(process) = &mut self.embedded_process {
            match process.try_wait() {
                Ok(Some(_)) => {
                    // Process has exited
                    self.embedded_process = None;
                    self.is_playing = false;
                    false
                }
                Ok(None) => true, // Still running
                Err(_) => {
                    self.embedded_process = None;
                    self.is_playing = false;
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Drop for VideoPreview {
    fn drop(&mut self) {
        self.stop();
    }
}
