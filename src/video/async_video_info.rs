use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashMap;
use tokio::sync::mpsc;
use crate::video::processor::{VideoProcessor, VideoInfo};

/// Request to load video info for a file
#[derive(Debug, Clone)]
pub struct VideoInfoRequest {
    pub file_path: PathBuf,
    pub request_id: u64,
}

/// Result of video info loading
#[derive(Debug, Clone)]
pub struct VideoInfoResult {
    pub request_id: u64,
    pub file_path: PathBuf,
    pub result: Result<VideoInfo, String>,
}

/// Asynchronous video info loader that runs FFmpeg in background threads
pub struct AsyncVideoInfoLoader {
    request_sender: mpsc::UnboundedSender<VideoInfoRequest>,
    result_receiver: Arc<Mutex<mpsc::UnboundedReceiver<VideoInfoResult>>>,
    next_request_id: Arc<Mutex<u64>>,
}

impl AsyncVideoInfoLoader {
    pub fn new() -> Self {
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<VideoInfoRequest>();
        let (result_tx, result_rx) = mpsc::unbounded_channel::<VideoInfoResult>();
        
        // Spawn worker thread that processes video info requests
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create async runtime");
            
            rt.block_on(async {
                while let Some(request) = request_rx.recv().await {
                    let result_tx = result_tx.clone();
                    let request_clone = request.clone();
                    
                    // Spawn background task for each FFmpeg call
                    tokio::task::spawn_blocking(move || {
                        log::debug!("Loading video info for: {:?}", request_clone.file_path);
                        
                        let result = match VideoProcessor::get_video_info(&request_clone.file_path) {
                            Ok(info) => {
                                log::debug!("Successfully loaded video info for: {:?} (duration: {:.2}s)", 
                                    request_clone.file_path, info.duration);
                                Ok(info)
                            }
                            Err(e) => {
                                log::debug!("Failed to load video info for: {:?} - {}", 
                                    request_clone.file_path, e);
                                Err(e.to_string())
                            }
                        };
                        
                        let response = VideoInfoResult {
                            request_id: request_clone.request_id,
                            file_path: request_clone.file_path,
                            result,
                        };
                        
                        // Send result back to main thread
                        if let Err(e) = result_tx.send(response) {
                            log::error!("Failed to send video info result: {}", e);
                        }
                    });
                }
            });
        });
        
        Self {
            request_sender: request_tx,
            result_receiver: Arc::new(Mutex::new(result_rx)),
            next_request_id: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Request video info for a file (non-blocking)
    pub fn request_video_info(&self, file_path: PathBuf) -> u64 {
        let request_id = {
            let mut id = self.next_request_id.lock().unwrap();
            *id += 1;
            *id
        };
        
        let request = VideoInfoRequest {
            file_path,
            request_id,
        };
        
        if let Err(e) = self.request_sender.send(request) {
            log::error!("Failed to send video info request: {}", e);
        }
        
        request_id
    }
    
    /// Get completed video info results (non-blocking)
    pub fn get_completed_results(&self) -> Vec<VideoInfoResult> {
        let mut results = Vec::new();
        
        if let Ok(mut receiver) = self.result_receiver.lock() {
            while let Ok(result) = receiver.try_recv() {
                results.push(result);
            }
        }
        
        results
    }
}

/// Manager for tracking pending video info requests and results
pub struct VideoInfoManager {
    loader: AsyncVideoInfoLoader,
    pending_requests: HashMap<PathBuf, u64>,
}

impl VideoInfoManager {
    pub fn new() -> Self {
        Self {
            loader: AsyncVideoInfoLoader::new(),
            pending_requests: HashMap::new(),
        }
    }
    
    /// Request video info for a file if not already pending
    pub fn request_if_needed(&mut self, file_path: PathBuf) -> bool {
        if self.pending_requests.contains_key(&file_path) {
            return false; // Already pending
        }
        
        let request_id = self.loader.request_video_info(file_path.clone());
        self.pending_requests.insert(file_path, request_id);
        true
    }
    
    /// Process completed results and return them
    pub fn process_completed(&mut self) -> Vec<VideoInfoResult> {
        let results = self.loader.get_completed_results();
        
        // Remove completed requests from pending
        for result in &results {
            self.pending_requests.remove(&result.file_path);
        }
        
        results
    }
    
    /// Check if a file has a pending request
    pub fn is_pending(&self, file_path: &PathBuf) -> bool {
        self.pending_requests.contains_key(file_path)
    }
}
