use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use tokio::sync::broadcast;
use chrono::{DateTime, Local};

#[derive(Debug, Clone)]
pub struct NewReplayFile {
    pub path: PathBuf,
    pub timestamp: DateTime<Local>,
}

pub struct FileMonitor {
    _watcher: RecommendedWatcher,
    event_sender: broadcast::Sender<NewReplayFile>,
}

impl FileMonitor {
    pub fn new(directory: &Path) -> anyhow::Result<(Self, broadcast::Receiver<NewReplayFile>)> {
        let (tx, rx) = mpsc::channel();
        let (event_sender, event_receiver) = broadcast::channel(32);
        
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(directory, RecursiveMode::NonRecursive)?;

        let event_sender_clone = event_sender.clone();
        thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                if let Ok(event) = event {
                    if let Event { kind: notify::EventKind::Create(_), paths, .. } = event {
                        for path in paths {
                            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                                if filename.starts_with("Replay ") && filename.ends_with(".mkv") {
                                    // Extract timestamp from filename
                                    if let Ok(timestamp) = crate::core::Clip::extract_timestamp_from_filename(&path) {
                                        let new_file = NewReplayFile {
                                            path: path.clone(),
                                            timestamp,
                                        };
                                        if let Err(e) = event_sender_clone.send(new_file) {
                                            log::error!("Failed to send file event for {:?}: {}", path, e);
                                        }
                                        log::info!("New replay file detected: {}", filename);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok((
            FileMonitor {
                _watcher: watcher,
                event_sender,
            },
            event_receiver,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NewReplayFile> {
        self.event_sender.subscribe()
    }
    
    pub fn scan_existing_files(directory: &Path) -> anyhow::Result<Vec<NewReplayFile>> {
        let mut files = Vec::new();
        
        if directory.exists() && directory.is_dir() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                
                if path.is_file() {
                    if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                        if filename.starts_with("Replay ") && filename.ends_with(".mkv") {
                            if let Ok(timestamp) = crate::core::Clip::extract_timestamp_from_filename(&path) {
                                files.push(NewReplayFile {
                                    path,
                                    timestamp,
                                });
                            }
                        }
                    }
                }
            }
        }
        
        // Sort by timestamp (newest first)
        files.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        
        Ok(files)
    }
}
