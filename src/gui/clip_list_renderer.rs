use egui;
use crate::core::Clip;
use crate::video::HoverThumbnailManager;
use std::path::PathBuf;

pub struct ClipListRenderer;

impl ClipListRenderer {
    /// Render a single clip item and return what actions need to be taken
    pub fn render_clip_item(
        ui: &mut egui::Ui,
        clip: &Clip,
        clip_index: usize,
        is_selected: bool,
        hover_thumbnail_manager: &mut HoverThumbnailManager,
        current_hover_target: &Option<PathBuf>,
    ) -> ClipRenderResult {
        let mut result = ClipRenderResult::default();
        
        let is_valid = clip.is_video_valid();
        
        // Make the entire container clickable and take full width
        let container_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::Vec2::new(ui.available_width(), 10.0 + ui.text_style_height(&egui::TextStyle::Body) * 3.0)
        );
        
        let is_visible = ui.clip_rect().intersects(container_rect);
        
        // Detect hover FIRST, before any UI interactions
        let mouse_pos = ui.input(|i| i.pointer.hover_pos());
        let is_hovering = if let Some(pos) = mouse_pos {
            container_rect.contains(pos) && is_valid
        } else {
            false
        };
        
        // Handle hover state changes
        if is_hovering {
            if current_hover_target.as_ref() != Some(&clip.original_file) {
                result.start_hover = Some(clip.original_file.clone());
            }
        } else if !is_hovering && current_hover_target.as_ref() == Some(&clip.original_file) {
            result.stop_hover = true;
        }
        
        // Create click interaction
        let container_response = ui.interact(container_rect, egui::Id::new(format!("clip_container_{}", clip_index)), egui::Sense::click());
        
        if container_response.clicked() && is_valid {
            result.clicked = true;
        }
        
        // Draw the container background
        if is_selected {
            ui.painter().rect_filled(container_rect, 4.0, ui.visuals().selection.bg_fill);
        } else if is_hovering {
            let mut hover_color = ui.visuals().selection.bg_fill;
            hover_color[3] = (hover_color[3] as f32 * 0.3) as u8;
            ui.painter().rect_filled(container_rect, 4.0, hover_color);
        }
        
        if is_selected {
            ui.painter().rect_stroke(container_rect, 4.0, ui.visuals().selection.stroke);
        }
        
        // Get thumbnail data
        let thumbnail_data = if is_hovering {
            if let Some(handle) = hover_thumbnail_manager.get_current_hover_thumbnail(ui.ctx()) {
                let texture_size = handle.size();
                let texture_id = handle.id();
                let frame_info = hover_thumbnail_manager.get_current_frame_info();
                Some((texture_id, texture_size, frame_info))
            } else {
                None
            }
        } else {
            if hover_thumbnail_manager.has_thumbnails(&clip.original_file) {
                hover_thumbnail_manager.get_first_thumbnail(&clip.original_file, ui.ctx()).map(|handle| {
                    let texture_size = handle.size();
                    let texture_id = handle.id();
                    (texture_id, texture_size, Some((0u8, 0.0f64)))
                })
            } else {
                None
            }
        };
        
        // Content area
        let content_rect = container_rect.shrink(5.0);
        
        // Check what's needed BEFORE entering UI closures
        result.needs_video_info = clip.video_length_seconds.is_none();
        
        // Request thumbnails directly for visible clips if cache not full
        if is_visible 
            && clip.video_length_seconds.is_some()
            && !hover_thumbnail_manager.has_thumbnails(&clip.original_file)
            && !hover_thumbnail_manager.is_generating(&clip.original_file)
            && !hover_thumbnail_manager.is_cache_full() {
            if let Some(duration) = clip.video_length_seconds {
                if duration >= 1.0 {
                    hover_thumbnail_manager.request_hover_thumbnails(
                        clip.original_file.clone(),
                        duration
                    );
                }
            }
        }
        
        // Evict thumbnails for clips outside viewport (unless currently hovering)
        if !is_visible 
            && hover_thumbnail_manager.has_thumbnails(&clip.original_file)
            && current_hover_target.as_ref() != Some(&clip.original_file) {
            hover_thumbnail_manager.evict_thumbnails(&clip.original_file);
        }
        
        ui.allocate_ui_at_rect(content_rect, |ui| {
            ui.horizontal(|ui| {
                // Thumbnail area
                Self::render_thumbnail(ui, thumbnail_data, is_hovering);
                
                ui.add_space(8.0);
                
                // Clip info
                ui.vertical(|ui| {
                    ui.scope(|ui| {
                        if !is_valid {
                            ui.visuals_mut().override_text_color = Some(egui::Color32::GRAY);
                        }
                        
                        ui.label(&clip.get_output_filename());
                        
                        if let Some(video_length) = clip.video_length_seconds {
                            if video_length >= 1.0 {
                                ui.small(format!("Original: {}", Clip::format_duration(video_length)));
                                if clip.has_target_duration() {
                                    ui.small(format!("Target: {}", Clip::format_duration(clip.target_duration_seconds as f64)));
                                }
                            } else {
                                ui.small("Waiting...");
                            }
                        } else {
                            ui.small("Waiting...");
                        }
                    });
                });
            });
        });
        
        ui.advance_cursor_after_rect(container_rect);
        ui.add_space(4.0);
        
        result
    }
    
    fn render_thumbnail(
        ui: &mut egui::Ui,
        thumbnail_data: Option<(egui::TextureId, [usize; 2], Option<(u8, f64)>)>,
        is_hovering: bool,
    ) {
        let thumbnail_width = 80.0;
        let thumbnail_height = 45.0;
        let thumbnail_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::Vec2::new(thumbnail_width, thumbnail_height)
        );
        
        ui.painter().rect_filled(thumbnail_rect, 4.0, egui::Color32::DARK_GRAY);
        
        if let Some((texture_id, texture_size, frame_info)) = thumbnail_data {
            let texture_aspect = texture_size[0] as f32 / texture_size[1] as f32;
            let container_aspect = thumbnail_width / thumbnail_height;
            
            let (image_width, image_height) = if texture_aspect > container_aspect {
                (thumbnail_width, thumbnail_width / texture_aspect)
            } else {
                (thumbnail_height * texture_aspect, thumbnail_height)
            };
            
            let image_x = thumbnail_rect.min.x + (thumbnail_width - image_width) * 0.5;
            let image_y = thumbnail_rect.min.y + (thumbnail_height - image_height) * 0.5;
            
            let image_rect = egui::Rect::from_min_size(
                egui::Pos2::new(image_x, image_y),
                egui::Vec2::new(image_width, image_height)
            );
            
            ui.painter().image(
                texture_id, 
                image_rect, 
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), 
                egui::Color32::WHITE
            );
            
            if is_hovering {
                if let Some((percentage, _timestamp)) = frame_info {
                    let frame_info = format!("{}%", percentage);
                    ui.painter().text(
                        egui::Pos2::new(thumbnail_rect.min.x + 2.0, thumbnail_rect.max.y - 12.0),
                        egui::Align2::LEFT_BOTTOM,
                        frame_info,
                        egui::FontId::proportional(10.0),
                        egui::Color32::WHITE
                    );
                }
            }
        }
        
        ui.allocate_space(egui::Vec2::new(thumbnail_width, thumbnail_height));
    }
}

#[derive(Default)]
pub struct ClipRenderResult {
    pub clicked: bool,
    pub start_hover: Option<PathBuf>,
    pub stop_hover: bool,
    pub needs_video_info: bool,
}
