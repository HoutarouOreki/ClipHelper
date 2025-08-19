use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use global_hotkey::hotkey::{Code, Modifiers};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub modifiers: String, // "Ctrl", "Alt", "Shift", "Ctrl+Alt", etc.
    pub key: String,       // "Numpad1", "F1", "A", etc.
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            modifiers: "Ctrl".to_string(),
            key: "Numpad1".to_string(),
        }
    }
}

impl HotkeyConfig {
    pub fn to_global_hotkey(&self) -> anyhow::Result<(Option<Modifiers>, Code)> {
        let modifiers = self.parse_modifiers()?;
        let code = self.parse_code()?;
        Ok((modifiers, code))
    }
    
    fn parse_modifiers(&self) -> anyhow::Result<Option<Modifiers>> {
        let parts: Vec<&str> = self.modifiers.split('+').collect();
        let mut result = Modifiers::empty();
        
        for part in parts {
            match part.trim() {
                "Ctrl" => result |= Modifiers::CONTROL,
                "Alt" => result |= Modifiers::ALT,
                "Shift" => result |= Modifiers::SHIFT,
                "Super" | "Win" => result |= Modifiers::SUPER,
                "" => {}, // Allow empty for no modifiers
                _ => return Err(anyhow::anyhow!("Unknown modifier: {}", part)),
            }
        }
        
        if result.is_empty() {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }
    
    fn parse_code(&self) -> anyhow::Result<Code> {
        match self.key.as_str() {
            // Numpad keys
            "Numpad0" => Ok(Code::Numpad0),
            "Numpad1" => Ok(Code::Numpad1),
            "Numpad2" => Ok(Code::Numpad2),
            "Numpad3" => Ok(Code::Numpad3),
            "Numpad4" => Ok(Code::Numpad4),
            "Numpad5" => Ok(Code::Numpad5),
            "Numpad6" => Ok(Code::Numpad6),
            "Numpad7" => Ok(Code::Numpad7),
            "Numpad8" => Ok(Code::Numpad8),
            "Numpad9" => Ok(Code::Numpad9),
            // Regular digits
            "Digit0" => Ok(Code::Digit0),
            "Digit1" => Ok(Code::Digit1),
            "Digit2" => Ok(Code::Digit2),
            "Digit3" => Ok(Code::Digit3),
            "Digit4" => Ok(Code::Digit4),
            "Digit5" => Ok(Code::Digit5),
            "Digit6" => Ok(Code::Digit6),
            "Digit7" => Ok(Code::Digit7),
            "Digit8" => Ok(Code::Digit8),
            "Digit9" => Ok(Code::Digit9),
            // Function keys
            "F1" => Ok(Code::F1),
            "F2" => Ok(Code::F2),
            "F3" => Ok(Code::F3),
            "F4" => Ok(Code::F4),
            "F5" => Ok(Code::F5),
            "F6" => Ok(Code::F6),
            "F7" => Ok(Code::F7),
            "F8" => Ok(Code::F8),
            "F9" => Ok(Code::F9),
            "F10" => Ok(Code::F10),
            "F11" => Ok(Code::F11),
            "F12" => Ok(Code::F12),
            _ => Err(anyhow::anyhow!("Unknown key code: {}", self.key)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub obs_replay_directory: PathBuf,
    pub output_directory: PathBuf,
    pub deleted_directory: PathBuf,
    pub trimmed_directory: PathBuf,
    pub last_watched_directory: Option<PathBuf>,
    pub ffmpeg_path: Option<PathBuf>,
    pub hotkeys: HashMap<String, HotkeyConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut hotkeys = HashMap::new();
        
        // Default hotkeys using numpad
        hotkeys.insert("clip_15s".to_string(), HotkeyConfig {
            modifiers: "Ctrl".to_string(),
            key: "Numpad1".to_string(),
        });
        hotkeys.insert("clip_30s".to_string(), HotkeyConfig {
            modifiers: "Ctrl".to_string(),
            key: "Numpad2".to_string(),
        });
        hotkeys.insert("clip_1m".to_string(), HotkeyConfig {
            modifiers: "Ctrl".to_string(),
            key: "Numpad3".to_string(),
        });
        hotkeys.insert("clip_2m".to_string(), HotkeyConfig {
            modifiers: "Ctrl".to_string(),
            key: "Numpad4".to_string(),
        });
        hotkeys.insert("clip_5m".to_string(), HotkeyConfig {
            modifiers: "Ctrl".to_string(),
            key: "Numpad5".to_string(),
        });
        
        Self {
            obs_replay_directory: PathBuf::from("./replays"),
            output_directory: PathBuf::from("./output"),
            deleted_directory: PathBuf::from("./output/deleted"),
            trimmed_directory: PathBuf::from("./output/trimmed"),
            last_watched_directory: None,
            ffmpeg_path: None,
            hotkeys,
        }
    }
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::config_path();
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| anyhow::anyhow!("Failed to read config file at {}: {}", config_path.display(), e))?;
            
            // Try to parse the config, but if it fails due to missing fields, create a new one
            match serde_json::from_str::<Self>(&content) {
                Ok(config) => {
                    log::info!("Loaded existing config from {}", config_path.display());
                    Ok(config)
                }
                Err(e) => {
                    log::warn!("Config file exists but has issues ({}), creating new one with defaults", e);
                    let new_config = Self::default();
                    new_config.save()
                        .map_err(|save_err| anyhow::anyhow!("Failed to save new config: {}", save_err))?;
                    log::info!("Created new config file at {}", config_path.display());
                    Ok(new_config)
                }
            }
        } else {
            log::info!("No config file found, creating default config");
            let config = Self::default();
            config.save()
                .map_err(|e| anyhow::anyhow!("Failed to save default config: {}", e))?;
            log::info!("Created new config file at {}", config_path.display());
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
        log::debug!("Ensuring directories exist...");
        
        if let Err(e) = std::fs::create_dir_all(&self.output_directory) {
            log::error!("Failed to create output directory {}: {}", self.output_directory.display(), e);
            return Err(anyhow::anyhow!("Failed to create output directory {}: {}", self.output_directory.display(), e));
        }
        log::debug!("Output directory ensured: {}", self.output_directory.display());
        
        if let Err(e) = std::fs::create_dir_all(&self.deleted_directory) {
            log::error!("Failed to create deleted directory {}: {}", self.deleted_directory.display(), e);
            return Err(anyhow::anyhow!("Failed to create deleted directory {}: {}", self.deleted_directory.display(), e));
        }
        log::debug!("Deleted directory ensured: {}", self.deleted_directory.display());
        
        if let Err(e) = std::fs::create_dir_all(&self.trimmed_directory) {
            log::error!("Failed to create trimmed directory {}: {}", self.trimmed_directory.display(), e);
            return Err(anyhow::anyhow!("Failed to create trimmed directory {}: {}", self.trimmed_directory.display(), e));
        }
        log::debug!("Trimmed directory ensured: {}", self.trimmed_directory.display());
        
        log::info!("All directories ensured successfully");
        Ok(())
    }
}
