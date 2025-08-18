use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use tokio::sync::broadcast;
use crate::hotkeys::{HotkeyEvent, HotkeyId};

pub struct HotkeyManager {
    _manager: GlobalHotKeyManager, // Keep reference alive
    event_sender: broadcast::Sender<HotkeyEvent>,
}

impl HotkeyManager {
    pub fn new() -> anyhow::Result<(Self, broadcast::Receiver<HotkeyEvent>)> {
        let manager = GlobalHotKeyManager::new()?;
        let (event_sender, event_receiver) = broadcast::channel(32);
        
        // Register hotkeys: Ctrl+1 through Ctrl+5
        let hotkeys = [
            (HotkeyId::Clip15s, Code::Digit1),
            (HotkeyId::Clip30s, Code::Digit2),
            (HotkeyId::Clip1m, Code::Digit3),
            (HotkeyId::Clip2m, Code::Digit4),
            (HotkeyId::Clip5m, Code::Digit5),
        ];

        for (_hotkey_id, code) in hotkeys {
            let hotkey = HotKey::new(Some(Modifiers::CONTROL), code);
            manager.register(hotkey)?;
        }

        Ok((
            HotkeyManager {
                _manager: manager,
                event_sender,
            },
            event_receiver,
        ))
    }

    pub fn process_events(&self) {
        // For now, just create events manually for testing
        // TODO: Fix the actual hotkey event processing once we resolve the API issues
        // The hotkey processing will need to be updated based on the exact global-hotkey version
        log::debug!("Processing hotkey events...");
    }

    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<HotkeyEvent> {
        self.event_sender.subscribe()
    }
}
