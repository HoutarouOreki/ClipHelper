use std::path::{Path, PathBuf};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use lru::LruCache;
use std::num::NonZeroUsize;
use anyhow::Result;
use log;

// Thumbnail dimensions - maximum size while preserving aspect ratio
const THUMBNAIL_MAX_WIDTH: u32 = 480;
const THUMBNAIL_MAX_HEIGHT: u32 = 360;
const THUMBNAIL_CHANNELS: usize = 4; // RGBA
// Note: Actual buffer size will vary based on video aspect ratio, so we'll allocate dynamically

/// Smart thumbnail cache with LRU eviction and async generation
pub struct SmartThumbnailCache {
    /// LRU cache of loaded textures (max 15 total across all videos)
    texture_cache: Arc<Mutex<LruCache<String, CachedThumbnail>>>,
    /// Track pending generation requests to prevent duplicates
    pending_requests: Arc<Mutex<HashSet<String>>>,
    /// Background worker for thumbnail generation
    generation_sender: mpsc::Sender<ThumbnailJob>,
    /// Results from background generation
    result_receiver: Arc<Mutex<mpsc::Receiver<ThumbnailResult>>>,
    /// Temporary directory for intermediate files
    temp_dir: PathBuf,
}

/// Cached thumbnail with metadata
#[derive(Clone)]
pub struct CachedThumbnail {
    pub texture_handle: egui::TextureHandle,
    pub timestamp: f64,
    pub generated_at: Instant,
}

/// Background job for thumbnail generation
#[derive(Debug)]
struct ThumbnailJob {
    video_path: PathBuf,
    timestamp: f64,
    cache_key: String,
}

/// Result from background thumbnail generation
#[derive(Debug)]
struct ThumbnailResult {
    cache_key: String,
    image_data: Option<Vec<u8>>, // RGBA bytes
    width: u32,
    height: u32,
    timestamp: f64,
    error: Option<String>,
}

impl SmartThumbnailCache {
    pub fn new() -> Result<Self> {
        let temp_dir = std::env::temp_dir().join("clip-helper-smart-thumbnails");
        std::fs::create_dir_all(&temp_dir)?;
        
        // Create LRU cache with capacity for 15 thumbnails total
        let texture_cache = Arc::new(Mutex::new(
            LruCache::new(NonZeroUsize::new(15).unwrap())
        ));
        let pending_requests = Arc::new(Mutex::new(HashSet::new()));
        
        let (job_sender, job_receiver) = mpsc::channel::<ThumbnailJob>();
        let (result_sender, result_receiver) = mpsc::channel::<ThumbnailResult>();
        let result_receiver = Arc::new(Mutex::new(result_receiver));
        
        // Background worker thread for thumbnail generation
        let worker_temp_dir = temp_dir.clone();
        thread::spawn(move || {
            Self::thumbnail_worker(job_receiver, result_sender, worker_temp_dir);
        });
        
        Ok(Self {
            texture_cache,
            pending_requests,
            generation_sender: job_sender,
            result_receiver,
            temp_dir,
        })
    }
    
    /// Generate cache key for video + timestamp
    fn generate_cache_key(video_path: &Path, timestamp: f64) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        video_path.hash(&mut hasher);
        ((timestamp * 10.0).round() as u64).hash(&mut hasher); // Round to 0.1s precision
        
        format!("thumb_{}_{:.1}", hasher.finish(), timestamp)
    }
    
    /// Request thumbnail - returns immediately if cached, starts generation if not
    pub fn request_thumbnail(&self, video_path: &Path, timestamp: f64) -> Option<CachedThumbnail> {
        let cache_key = Self::generate_cache_key(video_path, timestamp);
        
        // Check if already in cache
        if let Ok(mut cache) = self.texture_cache.lock() {
            if let Some(cached) = cache.get(&cache_key) {
                return Some(cached.clone());
            }
        }
        
        // Check if generation is already pending
        if let Ok(pending) = self.pending_requests.lock() {
            if pending.contains(&cache_key) {
                return None; // Generation in progress
            }
        }
        
        // Start generation
        if let Ok(mut pending) = self.pending_requests.lock() {
            pending.insert(cache_key.clone());
        }
        
        let job = ThumbnailJob {
            video_path: video_path.to_path_buf(),
            timestamp,
            cache_key,
        };
        
        let _ = self.generation_sender.send(job);
        None // Will be available in future frames
    }
    
    /// Get thumbnail if available in cache
    pub fn get_cached_thumbnail(&self, video_path: &Path, timestamp: f64) -> Option<CachedThumbnail> {
        let cache_key = Self::generate_cache_key(video_path, timestamp);
        
        if let Ok(mut cache) = self.texture_cache.lock() {
            cache.get(&cache_key).cloned()
        } else {
            None
        }
    }
    
    /// Pre-cache thumbnails around a timestamp (predictive caching)
    pub fn precache_around_timestamp(&self, video_path: &Path, center_timestamp: f64, duration: f64) {
        // Reduced predictive caching: only ±5s to reduce spam
        let timestamps = [
            (center_timestamp - 5.0).max(0.0),
            (center_timestamp + 5.0).min(duration),
        ];
        
        for &timestamp in &timestamps {
            self.request_thumbnail(video_path, timestamp);
        }
    }
    
    /// Process any completed thumbnail generation (call from UI thread)
    pub fn process_completed_thumbnails(&self, ctx: &egui::Context) {
        // Process all available results without blocking
        while let Ok(result_receiver) = self.result_receiver.lock() {
            match result_receiver.try_recv() {
                Ok(result) => {
                    // Remove from pending
                    if let Ok(mut pending) = self.pending_requests.lock() {
                        pending.remove(&result.cache_key);
                    }
                    
                    if let Some(image_data) = result.image_data {
                        self.create_texture_from_data(ctx, &result.cache_key, image_data, result.width, result.height, result.timestamp);
                    } else if let Some(error) = result.error {
                        log::error!("Thumbnail generation failed for {}: {}", result.cache_key, error);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break, // No more results
                Err(mpsc::TryRecvError::Disconnected) => break, // Worker thread died
            }
        }
    }
    
    /// Background worker for generating thumbnails
    fn thumbnail_worker(
        job_receiver: mpsc::Receiver<ThumbnailJob>,
        result_sender: mpsc::Sender<ThumbnailResult>,
        temp_dir: PathBuf,
    ) {
        while let Ok(job) = job_receiver.recv() {
            // Try up to 3 times for transient failures (file being written, etc.)
            let mut result = Self::generate_thumbnail_data(&job.video_path, job.timestamp, &temp_dir);
            
            // Retry on file access errors (likely temporary)
            if let Err(ref e) = result {
                let error_str = e.to_string().to_lowercase();
                if error_str.contains("cannot find the file") || 
                   error_str.contains("cannot access video file") ||
                   error_str.contains("permission denied") {
                    // Wait briefly and retry
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    result = Self::generate_thumbnail_data(&job.video_path, job.timestamp, &temp_dir);
                    
                    // One more try after a longer wait
                    if result.is_err() {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        result = Self::generate_thumbnail_data(&job.video_path, job.timestamp, &temp_dir);
                    }
                }
            }
            
            let thumbnail_result = match result {
                Ok((image_data, width, height)) => ThumbnailResult {
                    cache_key: job.cache_key,
                    image_data: Some(image_data),
                    width,
                    height,
                    timestamp: job.timestamp,
                    error: None,
                },
                Err(e) => ThumbnailResult {
                    cache_key: job.cache_key,
                    image_data: None,
                    width: 0,
                    height: 0,
                    timestamp: job.timestamp,
                    error: Some(e.to_string()),
                },
            };
            
            if result_sender.send(thumbnail_result).is_err() {
                break; // Main thread dropped the receiver
            }
        }
    }
    
    /// Generate thumbnail image data (RGBA at variable dimensions)
    fn generate_thumbnail_data(video_path: &Path, timestamp: f64, temp_dir: &Path) -> Result<(Vec<u8>, u32, u32)> {
        // Check if video file exists and is accessible
        if !video_path.exists() {
            return Err(anyhow::anyhow!("Video file does not exist: {}", video_path.display()));
        }
        
        // Check if file is readable (not being written to)
        if let Err(e) = std::fs::File::open(video_path) {
            return Err(anyhow::anyhow!("Cannot access video file: {}", e));
        }
        
        let temp_file = temp_dir.join(format!("temp_thumb_{}_{}.jpg", 
            std::process::id() % 10000,
            (timestamp * 10.0) as u64));
        
        // Use FFmpeg to extract frame - optimized for performance
        let output = std::process::Command::new("ffmpeg")
            .arg("-hwaccel").arg("auto")  // Hardware acceleration
            .arg("-ss").arg(format!("{:.3}", timestamp))  // Seek BEFORE input for faster positioning
            .arg("-i").arg(video_path)
            .arg("-vframes").arg("1")
            .arg("-vf").arg(format!("scale={}:{}:force_original_aspect_ratio=decrease", THUMBNAIL_MAX_WIDTH, THUMBNAIL_MAX_HEIGHT))  // Scale preserving aspect ratio, no padding
            .arg("-q:v").arg("2")  // High quality
            .arg("-y")  // Overwrite
            .arg(&temp_file)
            .stderr(std::process::Stdio::piped()) // Capture stderr for better error messages
            .stdout(std::process::Stdio::null()) // Suppress stdout
            .output()?;
        
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("FFmpeg failed: {}", error));
        }
        
        // Load and convert to RGBA
        let img = image::open(&temp_file)?;
        let rgba_img = img.to_rgba8();
        let (width, height) = rgba_img.dimensions();
        let image_data = rgba_img.into_raw();
        
        // Clean up temp file
        let _ = std::fs::remove_file(&temp_file);
        
        // Return both image data and dimensions
        Ok((image_data, width, height))
    }
    
    /// Create texture from image data (call this from UI thread)
    fn create_texture_from_data(
        &self,
        ctx: &egui::Context,
        cache_key: &str,
        image_data: Vec<u8>,
        width: u32,
        height: u32,
        timestamp: f64,
    ) {
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &image_data,
        );
        
        let texture_handle = ctx.load_texture(
            cache_key,
            color_image,
            egui::TextureOptions::LINEAR,
        );
        
        let cached_thumbnail = CachedThumbnail {
            texture_handle,
            timestamp,
            generated_at: Instant::now(),
        };
        
        if let Ok(mut cache) = self.texture_cache.lock() {
            cache.put(cache_key.to_string(), cached_thumbnail);
        }
        
        log::debug!("Created texture for thumbnail: {} at {:.1}s", cache_key, timestamp);
        
        // Request UI repaint to show the new thumbnail
        ctx.request_repaint();
    }
    
    /// Cleanup old thumbnails (call periodically)
    pub fn cleanup_old_thumbnails(&self) {
        let cutoff = Instant::now() - Duration::from_secs(30);
        
        if let Ok(mut cache) = self.texture_cache.lock() {
            let keys_to_remove: Vec<String> = cache
                .iter()
                .filter(|(_, thumbnail)| thumbnail.generated_at < cutoff)
                .map(|(key, _)| key.clone())
                .collect();
            
            for key in keys_to_remove {
                cache.pop(&key);
                log::debug!("Cleaned up old thumbnail: {}", key);
            }
        }
    }
}
