use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashMap;
use tokio::sync::mpsc;
use egui::{Context, TextureHandle, ColorImage};
use lru::LruCache;
use std::num::NonZeroUsize;

/// Request to generate thumbnails for a video file
#[derive(Debug, Clone)]
pub struct ThumbnailRequest {
    pub file_path: PathBuf,
    pub duration: f64,
    pub request_id: u64,
    pub timestamps: Vec<f64>, // Specific timestamps to generate (0%, 10%, 20%, etc.)
}

/// A single thumbnail result
#[derive(Debug, Clone)]
pub struct ThumbnailFrame {
    pub timestamp: f64,
    pub percentage: u8, // 0-100
    pub image_data: Vec<u8>, // RGB image data
    pub width: u32,
    pub height: u32,
}

/// Result of thumbnail generation
#[derive(Debug, Clone)]
pub struct ThumbnailResult {
    pub request_id: u64,
    pub file_path: PathBuf,
    pub frames: Vec<ThumbnailFrame>,
    pub success: bool,
    pub error: Option<String>,
}

/// Manages hover thumbnails for video clips
pub struct HoverThumbnailManager {
    request_sender: mpsc::UnboundedSender<ThumbnailRequest>,
    result_receiver: Arc<Mutex<mpsc::UnboundedReceiver<ThumbnailResult>>>,
    next_request_id: Arc<Mutex<u64>>,
    pending_requests: HashMap<PathBuf, u64>,
    completed_thumbnails: LruCache<PathBuf, HoverThumbnailSet>,
    frame_change_timer: std::time::Instant,
    current_hover_file: Option<PathBuf>,
    current_frame_index: usize,
}

/// A complete set of hover thumbnails for a video file
pub struct HoverThumbnailSet {
    pub frames: Vec<ThumbnailFrame>,
    pub texture_handles: Vec<Option<TextureHandle>>,
    pub file_path: PathBuf,
    pub duration: f64,
}

impl HoverThumbnailManager {
    pub fn new() -> Self {
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<ThumbnailRequest>();
        let (result_tx, result_rx) = mpsc::unbounded_channel::<ThumbnailResult>();
        
        // Spawn worker thread for thumbnail generation
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create thumbnail runtime");
            
            rt.block_on(async {
                while let Some(request) = request_rx.recv().await {
                    let result_tx = result_tx.clone();
                    let request_clone = request.clone();
                    
                    // Spawn background task for thumbnail generation
                    tokio::task::spawn_blocking(move || {
                        log::debug!("Generating hover thumbnails for: {:?}", request_clone.file_path);
                        
                        let mut frames = Vec::new();
                        let mut success = true;
                        let mut error = None;
                        
                        // Generate all thumbnails in a single FFmpeg process
                        match Self::generate_all_thumbnails(&request_clone.file_path, &request_clone.timestamps) {
                            Ok(thumbnail_frames) => {
                                frames = thumbnail_frames;
                            }
                            Err(e) => {
                                log::warn!("Failed to generate thumbnails for {:?}: {}", 
                                    request_clone.file_path, e);
                                success = false;
                                error = Some(e.to_string());
                            }
                        }
                        
                        let result = ThumbnailResult {
                            request_id: request_clone.request_id,
                            file_path: request_clone.file_path,
                            frames,
                            success,
                            error,
                        };
                        
                        if let Err(e) = result_tx.send(result) {
                            log::error!("Failed to send thumbnail result: {}", e);
                        }
                    });
                }
            });
        });
        
        Self {
            request_sender: request_tx,
            result_receiver: Arc::new(Mutex::new(result_rx)),
            next_request_id: Arc::new(Mutex::new(0)),
            pending_requests: HashMap::new(),
            completed_thumbnails: LruCache::new(NonZeroUsize::new(30).unwrap()),
            frame_change_timer: std::time::Instant::now(),
            current_hover_file: None,
            current_frame_index: 0,
        }
    }
    
    /// Generate all thumbnails using sequential processes with cache checking
    fn generate_all_thumbnails(file_path: &PathBuf, timestamps: &[f64]) -> anyhow::Result<Vec<ThumbnailFrame>> {
        // Create persistent cache directory based on video file
        let cache_dir = Self::get_cache_dir_for_file(file_path)?;
        std::fs::create_dir_all(&cache_dir)?;
        
        let mut all_frames = Vec::new();
        
        // Check for existing thumbnails first, only generate missing ones
        for (i, &timestamp) in timestamps.iter().enumerate() {
            let percentage = (i * 10) as u8;
            let thumb_file = cache_dir.join(format!("thumb_{:02}.jpg", i));
            
            // Check if thumbnail already exists
            if thumb_file.exists() {
                // Load existing thumbnail
                match image::open(&thumb_file) {
                    Ok(img) => {
                        let rgb_img = img.to_rgb8();
                        let (width, height) = rgb_img.dimensions();
                        let image_data = rgb_img.into_raw();
                        
                        all_frames.push(ThumbnailFrame {
                            timestamp,
                            percentage,
                            image_data,
                            width,
                            height,
                        });
                        continue; // Skip generation for this thumbnail
                    }
                    Err(_) => {
                        // File exists but corrupted, delete and regenerate
                        let _ = std::fs::remove_file(&thumb_file);
                    }
                }
            }
            
            // Generate missing thumbnail
            let mut command = std::process::Command::new("ffmpeg");
            command
                .arg("-ss").arg(format!("{:.3}", timestamp))
                .arg("-i").arg(file_path.to_str().unwrap())
                .arg("-vframes").arg("1")
                .arg("-vf").arg("scale=160:90:force_original_aspect_ratio=decrease")
                .arg("-q:v").arg("2")
                .arg("-f").arg("image2")
                .arg("-update").arg("1")
                .arg("-y")
                .arg(&thumb_file)
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null());
            
            // Use the global FFmpeg manager to enforce process limits
            match crate::video::execute_ffmpeg(command) {
                Ok(output) => {
                    if !output.status.success() {
                        log::warn!("Thumbnail generation failed for timestamp {:.1}s", timestamp);
                        continue;
                    }
                }
                Err(e) => {
                    log::warn!("Failed to execute FFmpeg for thumbnail generation: {}", e);
                    continue;
                }
            }
            
            // Load the newly generated thumbnail
            if thumb_file.exists() {
                match image::open(&thumb_file) {
                    Ok(img) => {
                        let rgb_img = img.to_rgb8();
                        let (width, height) = rgb_img.dimensions();
                        let image_data = rgb_img.into_raw();
                        
                        all_frames.push(ThumbnailFrame {
                            timestamp,
                            percentage,
                            image_data,
                            width,
                            height,
                        });
                    }
                    Err(e) => {
                        log::warn!("Failed to load thumbnail {}: {}", thumb_file.display(), e);
                    }
                }
            }
        }
        
        if all_frames.is_empty() {
            return Err(anyhow::anyhow!("No thumbnails were successfully generated"));
        }
        
        Ok(all_frames)
    }
    
    /// Get cache directory for a video file
    fn get_cache_dir_for_file(file_path: &PathBuf) -> anyhow::Result<PathBuf> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        // Create hash of file path for cache directory name
        let mut hasher = DefaultHasher::new();
        file_path.hash(&mut hasher);
        let hash = hasher.finish();
        
        // Use a persistent cache directory in AppData
        let cache_base = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Failed to get cache directory"))?
            .join("clip-helper")
            .join("hover_thumbnails");
            
        Ok(cache_base.join(format!("video_{:x}", hash)))
    }

    /// Generate a single thumbnail using the old method
    fn generate_single_thumbnail(file_path: &PathBuf, timestamp: f64) -> anyhow::Result<(Vec<u8>, u32, u32)> {
        use std::process::Command;
        
        // Create temporary file for thumbnail (use same approach as smart_thumbnail.rs)
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("hover_thumb_{}_{}.jpg", 
            std::process::id() % 10000,
            (timestamp * 10.0) as u64
        ));
        
        // Use the same FFmpeg approach as smart_thumbnail.rs - proven to work
        let mut command = Command::new("ffmpeg");
        command
            .arg("-hwaccel").arg("auto")  // Hardware acceleration
            .arg("-ss").arg(format!("{:.3}", timestamp))  // Seek BEFORE input for faster positioning
            .arg("-i").arg(file_path.to_str().unwrap())
            .arg("-vframes").arg("1")
            .arg("-vf").arg("scale=160:90:force_original_aspect_ratio=decrease")  // Scale preserving aspect ratio, no padding
            .arg("-q:v").arg("2")  // High quality
            .arg("-y")  // Overwrite
            .arg(&temp_file)
            .stderr(std::process::Stdio::piped()) // Capture stderr for better error messages
            .stdout(std::process::Stdio::null()); // Suppress stdout
            
        let output = command.output()?;
            
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("FFmpeg thumbnail failed: {}", error));
        }
        
        // Check if temp file was created
        if !temp_file.exists() {
            return Err(anyhow::anyhow!("Timestamp {:.3}s is likely beyond video duration - temp file not created", timestamp));
        }
        
        // Load the generated image using the image crate
        let img = image::open(&temp_file)?;
        let rgb_img = img.to_rgb8();
        let (width, height) = rgb_img.dimensions();
        let rgb_data = rgb_img.into_raw();
        
        // Clean up temp file
        let _ = std::fs::remove_file(&temp_file);
        
        Ok((rgb_data, width, height))
    }
    
    /// Request hover thumbnails for a video file
    pub fn request_hover_thumbnails(&mut self, file_path: PathBuf, duration: f64) -> bool {
        if self.pending_requests.contains_key(&file_path) || 
           self.completed_thumbnails.contains(&file_path) {
            return false; // Already pending or completed
        }
        
        // Send request to background thread - it will check disk first, then generate if needed
        let request_id = {
            let mut id = self.next_request_id.lock().unwrap();
            *id += 1;
            *id
        };
        
        // Generate timestamps for 0%, 10%, 20%, ..., 100%
        // Add safety margin to ensure we don't go past video end
        let safe_duration = (duration - 1.0).max(0.1); // Leave 1 second margin, minimum 0.1s
        let timestamps: Vec<f64> = (0i32..=10)
            .map(|i| (i as f64 / 10.0) * safe_duration)
            .collect();
        
        let request = ThumbnailRequest {
            file_path: file_path.clone(),
            duration,
            request_id,
            timestamps,
        };
        
        if let Err(e) = self.request_sender.send(request) {
            log::error!("Failed to send thumbnail request: {}", e);
            return false;
        }
        
        self.pending_requests.insert(file_path.clone(), request_id);
        log::debug!("Requested hover thumbnails for: {:?}", file_path);
        true
    }
    
    /// Load thumbnails from cache if they exist (used internally by background thread)
    fn load_cached_thumbnails(file_path: &PathBuf, duration: f64) -> anyhow::Result<Vec<ThumbnailFrame>> {
        let cache_dir = Self::get_cache_dir_for_file(file_path)?;
        
        if !cache_dir.exists() {
            return Err(anyhow::anyhow!("Cache directory does not exist"));
        }
        
        let safe_duration = (duration - 1.0).max(0.1);
        let timestamps: Vec<f64> = (0i32..=10)
            .map(|i| (i as f64 / 10.0) * safe_duration)
            .collect();
        
        let mut cached_frames = Vec::new();
        
        // Check if all thumbnails exist
        for (i, &timestamp) in timestamps.iter().enumerate() {
            let percentage = (i * 10) as u8;
            let thumb_file = cache_dir.join(format!("thumb_{:02}.jpg", i));
            
            if !thumb_file.exists() {
                return Err(anyhow::anyhow!("Missing thumbnail file: {}", thumb_file.display()));
            }
            
            // Load the cached thumbnail
            let img = image::open(&thumb_file)?;
            let rgb_img = img.to_rgb8();
            let (width, height) = rgb_img.dimensions();
            let image_data = rgb_img.into_raw();
            
            cached_frames.push(ThumbnailFrame {
                timestamp,
                percentage,
                image_data,
                width,
                height,
            });
        }
        
        Ok(cached_frames)
    }
    
    /// Process cached result directly without going through channels
    /// Process completed thumbnail results
    pub fn process_completed(&mut self, ctx: &Context) {
        let mut results = Vec::new();
        
        if let Ok(mut receiver) = self.result_receiver.lock() {
            while let Ok(result) = receiver.try_recv() {
                results.push(result);
            }
        }
        
        for result in results {
            self.pending_requests.remove(&result.file_path);
            
            if result.success && !result.frames.is_empty() {
                // Convert frames to texture handles
                let mut texture_handles = Vec::new();
                
                for frame in &result.frames {
                    let color_image = ColorImage::from_rgb(
                        [frame.width as usize, frame.height as usize],
                        &frame.image_data
                    );
                    
                    let texture_handle = ctx.load_texture(
                        format!("hover_thumb_{}_{}", 
                            result.file_path.to_string_lossy(), 
                            frame.percentage),
                        color_image,
                        egui::TextureOptions::default()
                    );
                    
                    texture_handles.push(Some(texture_handle));
                }
                
                let last_timestamp = result.frames.last().map(|f| f.timestamp).unwrap_or(0.0);
                
                let thumbnail_set = HoverThumbnailSet {
                    frames: result.frames,
                    texture_handles,
                    file_path: result.file_path.clone(),
                    duration: last_timestamp,
                };
                
                let num_frames = thumbnail_set.frames.len();
                
                // Immediately mark as recently used to prevent instant eviction
                let _ = self.completed_thumbnails.get(&result.file_path);
                
                let file_path_debug = result.file_path.clone();
                
                self.completed_thumbnails.put(result.file_path.clone(), thumbnail_set);
                log::debug!("Generated {} hover thumbnails for: {:?}", 
                    num_frames, file_path_debug);
            } else {
                log::warn!("Failed to generate hover thumbnails for: {:?} - {:?}", 
                    result.file_path, result.error);
            }
        }
    }
    
    /// Start hovering over a specific file
    pub fn start_hover(&mut self, file_path: &PathBuf) {
        if self.current_hover_file.as_ref() != Some(file_path) {
            self.current_hover_file = Some(file_path.clone());
            self.current_frame_index = 0;
            self.frame_change_timer = std::time::Instant::now();
        }
    }
    
    /// Stop hovering
    pub fn stop_hover(&mut self) {
        if self.current_hover_file.is_some() {
            self.current_hover_file = None;
            self.current_frame_index = 0;
        }
    }
    
    /// Get the current hover thumbnail to display
    pub fn get_current_hover_thumbnail(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
        let current_file = self.current_hover_file.clone()?;
        
        // Ensure texture handles exist for cached thumbnails
        self.ensure_texture_handles_exist(&current_file, ctx);
        
        let thumbnail_set = self.completed_thumbnails.get(&current_file)?;
        
        // Ensure we have multiple frames to cycle through
        if thumbnail_set.frames.len() <= 1 {
            // If only one frame, just return it
            return thumbnail_set.texture_handles.get(0)?.as_ref();
        }
        
        // Update frame index every 0.5 seconds
        let elapsed = self.frame_change_timer.elapsed().as_millis();
        
        if elapsed >= 500 {
            self.current_frame_index = (self.current_frame_index + 1) % thumbnail_set.frames.len();
            self.frame_change_timer = std::time::Instant::now();
            
            // Force immediate repaint when frame changes
            ctx.request_repaint();
        }
        
        // Continuous repaints for smooth animation
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        
        thumbnail_set.texture_handles
            .get(self.current_frame_index)?
            .as_ref()
    }
    
    /// Ensure texture handles exist for a thumbnail set (create on-demand for cached thumbnails)
    fn ensure_texture_handles_exist(&mut self, file_path: &PathBuf, ctx: &egui::Context) {
        if let Some(thumbnail_set) = self.completed_thumbnails.get_mut(file_path) {
            // If texture handles are empty or fewer than frames, create them
            if thumbnail_set.texture_handles.len() < thumbnail_set.frames.len() {
                thumbnail_set.texture_handles.clear();
                
                for frame in &thumbnail_set.frames {
                    let color_image = ColorImage::from_rgb(
                        [frame.width as usize, frame.height as usize],
                        &frame.image_data
                    );
                    
                    let texture_handle = ctx.load_texture(
                        format!("hover_thumb_{}_{}", 
                            file_path.to_string_lossy(), 
                            frame.percentage),
                        color_image,
                        egui::TextureOptions::default()
                    );
                    
                    thumbnail_set.texture_handles.push(Some(texture_handle));
                }
            }
        }
    }
    
    /// Check if thumbnails are available for a file
    pub fn has_thumbnails(&self, file_path: &PathBuf) -> bool {
        self.completed_thumbnails.contains(file_path)
    }
    
    /// Check if thumbnails exist on disk (cheaper than loading them)
    pub fn thumbnails_exist_on_disk(file_path: &PathBuf) -> bool {
        if let Ok(cache_dir) = Self::get_cache_dir_for_file(file_path) {
            if cache_dir.exists() {
                // Check if at least the first thumbnail exists
                cache_dir.join("thumb_00.jpg").exists()
            } else {
                false
            }
        } else {
            false
        }
    }
    
    /// Check if thumbnails are being generated for a file
    pub fn is_generating(&self, file_path: &PathBuf) -> bool {
        self.pending_requests.contains_key(file_path)
    }
    
    pub fn is_cache_full(&self) -> bool {
        self.completed_thumbnails.len() >= self.completed_thumbnails.cap().get()
    }
    
    /// Manually evict thumbnails for a file from cache
    pub fn evict_thumbnails(&mut self, file_path: &PathBuf) {
        self.completed_thumbnails.pop(file_path);
    }
    
    /// Get current frame info for display
    pub fn get_current_frame_info(&mut self) -> Option<(u8, f64)> {
        let current_file = self.current_hover_file.as_ref()?;
        let thumbnail_set = self.completed_thumbnails.get(current_file)?;
        let frame = thumbnail_set.frames.get(self.current_frame_index)?;
        Some((frame.percentage, frame.timestamp))
    }
    
    /// Get the first thumbnail (for non-hover display)
    pub fn get_first_thumbnail(&mut self, file_path: &PathBuf, ctx: &egui::Context) -> Option<&TextureHandle> {
        // Ensure texture handles exist for cached thumbnails
        self.ensure_texture_handles_exist(file_path, ctx);
        
        let thumbnail_set = self.completed_thumbnails.get(file_path)?;
        thumbnail_set.texture_handles
            .get(0)?
            .as_ref()
    }
}
