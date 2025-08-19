use eframe::egui;
use crate::core::Clip;
use crate::video::VideoPreview;

pub struct TimelineWidget {
    pub scrub_position: f64,
    pub is_scrubbing: bool,
    pub zoom_level: f32,
}

impl TimelineWidget {
    pub fn new() -> Self {
        Self {
            scrub_position: 0.0,
            is_scrubbing: false,
            zoom_level: 1.0,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, clip: &mut Clip, video_preview: &mut Option<VideoPreview>) -> egui::Response {
        let duration = clip.duration_seconds as f64;
        let trim_start = clip.trim_start;
        let trim_end = clip.trim_end;
        
        let available_width = ui.available_width() - 40.0; // Leave margin for labels
        let timeline_height = 60.0;
        
        let (rect, response) = ui.allocate_exact_size(
            egui::Vec2::new(available_width, timeline_height),
            egui::Sense::click_and_drag()
        );
        
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            
            // Background
            painter.rect_filled(
                rect,
                egui::Rounding::same(4.0),
                ui.visuals().extreme_bg_color,
            );
            
            // Timeline track
            let track_rect = egui::Rect::from_min_size(
                rect.min + egui::Vec2::new(10.0, 20.0),
                egui::Vec2::new(available_width - 20.0, 20.0),
            );
            
            painter.rect_stroke(
                track_rect,
                egui::Rounding::same(2.0),
                egui::Stroke::new(1.0, ui.visuals().text_color()),
            );
            
            // Time markers
            let time_per_pixel = duration / track_rect.width() as f64;
            let marker_interval = self.calculate_marker_interval(time_per_pixel);
            
            for i in 0..((duration / marker_interval) as i32 + 1) {
                let time = i as f64 * marker_interval;
                if time <= duration {
                    let x = track_rect.min.x + ((time / duration) * track_rect.width() as f64) as f32;
                    
                    // Marker line
                    painter.line_segment(
                        [egui::Pos2::new(x, track_rect.min.y), egui::Pos2::new(x, track_rect.max.y)],
                        egui::Stroke::new(0.5, ui.visuals().weak_text_color()),
                    );
                    
                    // Time label
                    let time_text = self.format_time(time);
                    painter.text(
                        egui::Pos2::new(x, track_rect.min.y - 15.0),
                        egui::Align2::CENTER_BOTTOM,
                        time_text,
                        egui::FontId::monospace(10.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            
            // Trim region (selected area)
            let trim_start_x = track_rect.min.x + ((trim_start / duration) * track_rect.width() as f64) as f32;
            let trim_end_x = track_rect.min.x + ((trim_end / duration) * track_rect.width() as f64) as f32;
            
            let trim_rect = egui::Rect::from_min_max(
                egui::Pos2::new(trim_start_x, track_rect.min.y),
                egui::Pos2::new(trim_end_x, track_rect.max.y),
            );
            
            painter.rect_filled(
                trim_rect,
                egui::Rounding::same(2.0),
                ui.visuals().selection.bg_fill.gamma_multiply(0.5),
            );
            
            // Trim handles
            let handle_width = 8.0;
            let start_handle = egui::Rect::from_center_size(
                egui::Pos2::new(trim_start_x, track_rect.center().y),
                egui::Vec2::new(handle_width, track_rect.height() + 10.0),
            );
            let end_handle = egui::Rect::from_center_size(
                egui::Pos2::new(trim_end_x, track_rect.center().y),
                egui::Vec2::new(handle_width, track_rect.height() + 10.0),
            );
            
            painter.rect_filled(
                start_handle,
                egui::Rounding::same(4.0),
                ui.visuals().selection.bg_fill,
            );
            painter.rect_filled(
                end_handle,
                egui::Rounding::same(4.0),
                ui.visuals().selection.bg_fill,
            );
            
            // Current playback position
            if let Some(preview) = video_preview {
                let current_x = track_rect.min.x + ((preview.current_time / duration) * track_rect.width() as f64) as f32;
                painter.line_segment(
                    [egui::Pos2::new(current_x, rect.min.y), egui::Pos2::new(current_x, rect.max.y)],
                    egui::Stroke::new(2.0, egui::Color32::RED),
                );
                
                // Playhead
                let playhead_rect = egui::Rect::from_center_size(
                    egui::Pos2::new(current_x, rect.min.y + 5.0),
                    egui::Vec2::new(12.0, 10.0),
                );
                painter.rect_filled(
                    playhead_rect,
                    egui::Rounding::same(2.0),
                    egui::Color32::RED,
                );
            }
            
            // Handle interactions
            if response.clicked() || response.dragged() {
                if let Some(click_pos) = response.interact_pointer_pos() {
                    let click_x = click_pos.x;
                    let relative_x = ((click_x - track_rect.min.x) / track_rect.width()) as f64;
                    let clicked_time = relative_x * duration;
                    
                    // Check if clicking on trim handles
                    if response.clicked() {
                        if start_handle.contains(click_pos) {
                            // Clicked start handle
                            self.is_scrubbing = true;
                        } else if end_handle.contains(click_pos) {
                            // Clicked end handle
                            self.is_scrubbing = true;
                        } else {
                            // Clicked timeline - seek
                            if let Some(preview) = video_preview {
                                preview.seek_to(clicked_time);
                            }
                            self.scrub_position = clicked_time;
                        }
                    }
                    
                    // Handle dragging for trim adjustment
                    if response.dragged() && self.is_scrubbing {
                        let clamped_time = clicked_time.clamp(0.0, duration);
                        
                        // Determine which handle is closer
                        let dist_to_start = (clicked_time - trim_start).abs();
                        let dist_to_end = (clicked_time - trim_end).abs();
                        
                        if dist_to_start < dist_to_end {
                            // Adjust start trim
                            clip.trim_start = clamped_time.min(trim_end - 0.1);
                        } else {
                            // Adjust end trim
                            clip.trim_end = clamped_time.max(trim_start + 0.1);
                        }
                    }
                }
            }
            
            if response.drag_stopped() {
                self.is_scrubbing = false;
            }
            
            // Time display
            let current_time = if let Some(preview) = video_preview {
                preview.current_time
            } else {
                self.scrub_position
            };
            
            painter.text(
                rect.max - egui::Vec2::new(10.0, 5.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("{} / {}", self.format_time(current_time), self.format_time(duration)),
                egui::FontId::monospace(12.0),
                ui.visuals().text_color(),
            );
        }
        
        response
    }
    
    fn calculate_marker_interval(&self, time_per_pixel: f64) -> f64 {
        // Calculate appropriate time interval for markers based on zoom
        let target_pixel_spacing = 60.0; // Target pixels between markers
        let base_interval = time_per_pixel * target_pixel_spacing;
        
        // Round to nice intervals
        if base_interval <= 1.0 {
            0.5
        } else if base_interval <= 5.0 {
            1.0
        } else if base_interval <= 10.0 {
            5.0
        } else if base_interval <= 30.0 {
            10.0
        } else if base_interval <= 60.0 {
            30.0
        } else {
            60.0
        }
    }
    
    fn format_time(&self, seconds: f64) -> String {
        let mins = (seconds / 60.0) as u32;
        let secs = seconds % 60.0;
        format!("{}:{:04.1}", mins, secs)
    }
}
