pub struct VideoPreview {
    pub current_time: f64,
    pub is_playing: bool,
    pub total_duration: f64,
}

impl VideoPreview {
    pub fn new(duration: f64) -> Self {
        Self {
            current_time: 0.0,
            is_playing: false,
            total_duration: duration,
        }
    }

    pub fn seek_to(&mut self, time: f64) {
        self.current_time = time.clamp(0.0, self.total_duration);
    }

    pub fn skip_forward(&mut self, seconds: f64) {
        self.seek_to(self.current_time + seconds);
    }

    pub fn skip_backward(&mut self, seconds: f64) {
        self.seek_to(self.current_time - seconds);
    }

    pub fn play(&mut self) {
        self.is_playing = true;
    }

    pub fn pause(&mut self) {
        self.is_playing = false;
    }

    pub fn toggle_playback(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn goto_start(&mut self) {
        self.seek_to(0.0);
    }

    pub fn goto_last_5_seconds(&mut self) {
        self.seek_to((self.total_duration - 5.0).max(0.0));
    }
}
