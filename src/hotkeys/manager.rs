use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use tokio::sync::broadcast;
use crate::hotkeys::{HotkeyEvent, HotkeyId};
use crate::core::{AppConfig, HotkeyConfig};
use std::collections::HashMap;

pub struct HotkeyManager {
    _manager: GlobalHotKeyManager, // Keep reference alive
    event_sender: broadcast::Sender<HotkeyEvent>,
    hotkey_map: HashMap<u32, HotkeyId>,
}

impl HotkeyManager {
    pub fn new(config: &AppConfig) -> anyhow::Result<(Self, broadcast::Receiver<HotkeyEvent>)> {
        log::info!("Initializing HotkeyManager...");
        
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| anyhow::anyhow!("Failed to create GlobalHotKeyManager: {}", e))?;
        log::info!("GlobalHotKeyManager created successfully");
        
        let (event_sender, event_receiver) = broadcast::channel(32);
        let mut hotkey_map = HashMap::new();
        
        // Register hotkeys from config
        let hotkey_mappings = [
            ("clip_15s", HotkeyId::Clip15s, "15s clip"),
            ("clip_30s", HotkeyId::Clip30s, "30s clip"),
            ("clip_1m", HotkeyId::Clip1m, "1m clip"),
            ("clip_2m", HotkeyId::Clip2m, "2m clip"),
            ("clip_5m", HotkeyId::Clip5m, "5m clip"),
        ];

        log::info!("Registering {} global hotkeys...", hotkey_mappings.len());
        for (config_key, hotkey_id, description) in hotkey_mappings {
            if let Some(hotkey_config) = config.hotkeys.get(config_key) {
                match hotkey_config.to_global_hotkey() {
                    Ok((modifiers, code)) => {
                        let hotkey = HotKey::new(modifiers, code);
                        match manager.register(hotkey) {
                            Ok(_) => {
                                log::info!("Successfully registered {} -> {}", 
                                    format!("{}+{}", hotkey_config.modifiers, hotkey_config.key),
                                    description);
                                hotkey_map.insert(hotkey.id(), hotkey_id);
                            }
                            Err(e) => {
                                log::error!("Failed to register {} ({}+{}): {}", 
                                    description, hotkey_config.modifiers, hotkey_config.key, e);
                                return Err(anyhow::anyhow!("Failed to register {} ({}+{}): {}", 
                                    description, hotkey_config.modifiers, hotkey_config.key, e));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Invalid hotkey configuration for {}: {}", config_key, e);
                        return Err(anyhow::anyhow!("Invalid hotkey configuration for {}: {}", config_key, e));
                    }
                }
            } else {
                log::error!("Missing hotkey configuration for: {}", config_key);
                return Err(anyhow::anyhow!("Missing hotkey configuration for: {}", config_key));
            }
        }
        
        log::info!("All hotkeys registered successfully. Hotkey map size: {}", hotkey_map.len());

        Ok((
            HotkeyManager {
                _manager: manager,
                event_sender,
                hotkey_map,
            },
            event_receiver,
        ))
    }

    pub fn process_events(&self) {
        // Process all pending hotkey events
        let receiver = GlobalHotKeyEvent::receiver();
        let mut event_count = 0;
        
        while let Ok(event) = receiver.try_recv() {
            event_count += 1;
            log::debug!("Received hotkey event #{}: ID={}, state={:?}", event_count, event.id(), event.state());
            
            if event.state() == HotKeyState::Pressed {
                if let Some(&hotkey_id) = self.hotkey_map.get(&event.id()) {
                    let clip_duration = hotkey_id.to_clip_duration();
                    log::info!("Hotkey triggered: {:?} -> {}s clip", hotkey_id, clip_duration.clone() as u32);
                    
                    match self.event_sender.send(HotkeyEvent::ClipRequested(clip_duration)) {
                        Ok(_) => log::debug!("Hotkey event sent successfully"),
                        Err(e) => log::error!("Failed to send hotkey event: {}", e),
                    }
                } else {
                    log::warn!("Received hotkey event for unknown ID: {}", event.id());
                }
            }
        }
        
        if event_count > 0 {
            log::debug!("Processed {} hotkey events", event_count);
        }
    }

    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<HotkeyEvent> {
        self.event_sender.subscribe()
    }
}
