use crate::core::ClipDuration;

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    ClipRequested(ClipDuration),
}

#[derive(Debug, Clone, Copy)]
pub enum HotkeyId {
    Clip15s = 1,
    Clip30s = 2,
    Clip1m = 3,
    Clip2m = 4,
    Clip5m = 5,
}

impl HotkeyId {
    pub fn to_clip_duration(self) -> ClipDuration {
        match self {
            HotkeyId::Clip15s => ClipDuration::Seconds15,
            HotkeyId::Clip30s => ClipDuration::Seconds30,
            HotkeyId::Clip1m => ClipDuration::Minutes1,
            HotkeyId::Clip2m => ClipDuration::Minutes2,
            HotkeyId::Clip5m => ClipDuration::Minutes5,
        }
    }
}
