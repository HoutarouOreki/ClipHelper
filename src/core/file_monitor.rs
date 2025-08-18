use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use tokio::sync::broadcast;

pub struct FileMonitor {
    _watcher: RecommendedWatcher,
    event_sender: broadcast::Sender<String>,
}

impl FileMonitor {
    pub fn new(directory: &Path) -> anyhow::Result<(Self, broadcast::Receiver<String>)> {
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
                                    let _ = event_sender_clone.send(filename.to_string());
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

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.event_sender.subscribe()
    }
}
