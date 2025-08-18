use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub obs_replay_directory: PathBuf,
    pub output_directory: PathBuf,
    pub deleted_directory: PathBuf,
    pub trimmed_directory: PathBuf,
    pub ffmpeg_path: Option<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            obs_replay_directory: PathBuf::from("./replays"),
            output_directory: PathBuf::from("./output"),
            deleted_directory: PathBuf::from("./output/deleted"),
            trimmed_directory: PathBuf::from("./output/trimmed"),
            ffmpeg_path: None,
        }
    }
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::config_path();
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            let config = Self::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clip-helper")
            .join("config.json")
    }

    pub fn ensure_directories(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.output_directory)?;
        std::fs::create_dir_all(&self.deleted_directory)?;
        std::fs::create_dir_all(&self.trimmed_directory)?;
        Ok(())
    }
}
