use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use anyhow::Result;

/// Manages thumbnail generation and caching for video files
pub struct ThumbnailManager {
    cache_dir: PathBuf,
    cache: Arc<Mutex<HashMap<String, PathBuf>>>,
    pending_requests: Arc<Mutex<HashSet<String>>>,
    generation_sender: mpsc::Sender<ThumbnailRequest>,
}

#[derive(Debug, Clone)]
pub struct ThumbnailRequest {
    pub video_path: PathBuf,
    pub timestamp: f64,
    pub cache_key: String,
}

impl ThumbnailManager {
    pub fn new() -> Result<Self> {
        let cache_dir = std::env::temp_dir().join("clip-helper").join("thumbnails");
        std::fs::create_dir_all(&cache_dir)?;
        
        let (generation_sender, generation_receiver) = mpsc::channel::<ThumbnailRequest>();
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests = Arc::new(Mutex::new(HashSet::new()));
        
        // Start background worker thread
        let worker_cache = cache.clone();
        let worker_pending = pending_requests.clone();
        let worker_cache_dir = cache_dir.clone();
        thread::spawn(move || {
            while let Ok(request) = generation_receiver.recv() {
                let thumbnail_path = worker_cache_dir.join(&request.cache_key);
                
                // Generate thumbnail using FFmpeg
                let result = crate::video::VideoProcessor::extract_thumbnail(
                    &request.video_path,
                    request.timestamp,
                    &thumbnail_path,
                );

                match result {
                    Ok(_) => {
                        // Add to cache
                        if let Ok(mut cache_lock) = worker_cache.lock() {
                            cache_lock.insert(request.cache_key.clone(), thumbnail_path);
                        }
                        log::debug!("Generated thumbnail for {} at {:.3}s", 
                                  request.video_path.display(), request.timestamp);
                    }
                    Err(e) => {
                        log::error!("Failed to generate thumbnail: {}", e);
                    }
                }
                
                // Remove from pending requests when done
                if let Ok(mut pending) = worker_pending.lock() {
                    pending.remove(&request.cache_key);
                }
            }
        });
        
        Ok(Self {
            cache_dir,
            cache,
            pending_requests,
            generation_sender,
        })
    }

    /// Generate a cache key for a video file at a specific timestamp
    pub fn generate_cache_key(video_path: &Path, timestamp: f64) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        video_path.hash(&mut hasher);
        timestamp.to_bits().hash(&mut hasher);
        
        format!("thumb_{}_{:.3}.jpg", hasher.finish(), timestamp)
    }

    /// Request a thumbnail for a video at a specific timestamp
    pub fn request_thumbnail(&self, video_path: &Path, timestamp: f64) -> Result<String> {
        let cache_key = Self::generate_cache_key(video_path, timestamp);
        
        // Check if thumbnail already exists in cache
        if let Ok(cache) = self.cache.lock() {
            if let Some(thumbnail_path) = cache.get(&cache_key) {
                if thumbnail_path.exists() {
                    return Ok(cache_key);
                }
            }
        }

        // Check if request is already pending (prevent spam)
        if let Ok(pending) = self.pending_requests.lock() {
            if pending.contains(&cache_key) {
                return Ok(cache_key); // Return the key even if pending
            }
        }

        // Add to pending requests
        if let Ok(mut pending) = self.pending_requests.lock() {
            pending.insert(cache_key.clone());
        }

        // Request thumbnail generation
        let request = ThumbnailRequest {
            video_path: video_path.to_path_buf(),
            timestamp,
            cache_key: cache_key.clone(),
        };

        let _ = self.generation_sender.send(request);
        Ok(cache_key)
    }

    /// Get the path to a cached thumbnail
    pub fn get_thumbnail_path(&self, cache_key: &str) -> Option<PathBuf> {
        if let Ok(cache) = self.cache.lock() {
            cache.get(cache_key).cloned()
        } else {
            None
        }
    }

    /// Generate thumbnails at standard positions (0%, 25%, 50%, 75%, 100%)
    pub fn request_standard_thumbnails(&self, video_path: &Path, duration: f64) -> Vec<String> {
        let positions = [0.0, 0.25, 0.5, 0.75, 1.0];
        let mut cache_keys = Vec::new();

        for &pos in &positions {
            let timestamp = (duration * pos).min(duration - 0.1).max(0.0);
            if let Ok(key) = self.request_thumbnail(video_path, timestamp) {
                cache_keys.push(key);
            }
        }

        cache_keys
    }

    /// Clear old thumbnails to prevent cache bloat
    pub fn cleanup_old_thumbnails(&self, max_age_hours: u64) -> Result<()> {
        let now = std::time::SystemTime::now();
        let max_age = std::time::Duration::from_secs(max_age_hours * 3600);

        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() && path.extension().map_or(false, |ext| ext == "jpg") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(age) = now.duration_since(modified) {
                            if age > max_age {
                                let _ = std::fs::remove_file(&path);
                                log::debug!("Cleaned up old thumbnail: {}", path.display());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
