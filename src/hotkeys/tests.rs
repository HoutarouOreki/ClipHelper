#[cfg(test)]
mod tests {
    use super::super::{HotkeyId, HotkeyEvent};
    use crate::core::ClipDuration;

    #[test]
    fn test_hotkey_id_to_clip_duration() {
        assert_eq!(HotkeyId::Clip15s.to_clip_duration(), ClipDuration::Seconds15);
        assert_eq!(HotkeyId::Clip30s.to_clip_duration(), ClipDuration::Seconds30);
        assert_eq!(HotkeyId::Clip1m.to_clip_duration(), ClipDuration::Minutes1);
        assert_eq!(HotkeyId::Clip2m.to_clip_duration(), ClipDuration::Minutes2);
        assert_eq!(HotkeyId::Clip5m.to_clip_duration(), ClipDuration::Minutes5);
    }

    #[test]
    fn test_hotkey_event_creation() {
        let event = HotkeyEvent::ClipRequested(ClipDuration::Seconds15);
        match event {
            HotkeyEvent::ClipRequested(ClipDuration::Seconds15) => {
                // Test passes
            }
            _ => panic!("Unexpected event variant"),
        }
    }

    #[test]
    fn test_clip_duration_conversion() {
        assert_eq!(ClipDuration::Seconds15.clone() as u32, 15);
        assert_eq!(ClipDuration::Seconds30.clone() as u32, 30);
        assert_eq!(ClipDuration::Minutes1.clone() as u32, 60);
        assert_eq!(ClipDuration::Minutes2.clone() as u32, 120);
        assert_eq!(ClipDuration::Minutes5.clone() as u32, 300);
    }
}
