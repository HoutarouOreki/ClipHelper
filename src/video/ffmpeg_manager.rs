use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::process::{Command, Output};
use anyhow::Result;

/// Global FFmpeg process manager that enforces a maximum of 4 concurrent processes
pub struct FFmpegManager {
    active_count: Arc<AtomicUsize>,
}

impl FFmpegManager {
    const MAX_PROCESSES: usize = 4;
    
    pub fn new() -> Self {
        Self {
            active_count: Arc::new(AtomicUsize::new(0)),
        }
    }
    
    /// Execute an FFmpeg command, returning an error if we're at the limit
    pub fn execute_ffmpeg(&self, mut command: Command) -> Result<Output> {
        let current_count = self.active_count.load(Ordering::SeqCst);
        
        if current_count >= Self::MAX_PROCESSES {
            return Err(anyhow::anyhow!(
                "Cannot execute FFmpeg: {} processes already running (max: {})",
                current_count,
                Self::MAX_PROCESSES
            ));
        }
        
        // Increment counter before spawning
        self.active_count.fetch_add(1, Ordering::SeqCst);
        
        log::debug!("Executing FFmpeg process, active count: {}", 
            self.active_count.load(Ordering::SeqCst));
        
        // Execute the process
        let result = command.output();
        
        // Decrement counter after completion
        self.active_count.fetch_sub(1, Ordering::SeqCst);
        
        log::debug!("FFmpeg process completed, active count: {}", 
            self.active_count.load(Ordering::SeqCst));
        
        result.map_err(|e| anyhow::anyhow!("FFmpeg execution failed: {}", e))
    }
    
    /// Get current active process count
    pub fn active_count(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }
}

/// Global singleton instance
static FFMPEG_MANAGER: std::sync::OnceLock<FFmpegManager> = std::sync::OnceLock::new();

/// Get the global FFmpeg manager instance
pub fn get_ffmpeg_manager() -> &'static FFmpegManager {
    FFMPEG_MANAGER.get_or_init(|| FFmpegManager::new())
}

/// Convenience function to execute FFmpeg with the global manager
pub fn execute_ffmpeg(command: Command) -> Result<Output> {
    let manager = get_ffmpeg_manager();
    manager.execute_ffmpeg(command)
}
