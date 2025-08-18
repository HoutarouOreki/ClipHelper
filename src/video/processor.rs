use crate::core::Clip;
use std::path::Path;
use std::process::Command;

pub struct VideoProcessor;

impl VideoProcessor {
    pub fn trim_clip(clip: &Clip, output_path: &Path) -> anyhow::Result<()> {
        let start_time = format!("{:.3}", clip.trim_start);
        let duration = format!("{:.3}", clip.trim_end - clip.trim_start);
        
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-i")
            .arg(&clip.original_file)
            .arg("-ss")
            .arg(&start_time)
            .arg("-t")
            .arg(&duration)
            .arg("-c:v")
            .arg("copy"); // Copy video without re-encoding for speed

        // Handle audio tracks
        if !clip.audio_tracks.is_empty() {
            // Create mixed track (track 1)
            let mut filter_complex = String::new();
            let mut audio_inputs = Vec::new();
            
            for (i, track) in clip.audio_tracks.iter().enumerate() {
                if track.enabled {
                    if track.surround_mode {
                        // Map to surround left/right
                        audio_inputs.push(format!("[0:a:{}]channelmap=map=FL|FR[a{}]", track.index, i));
                    } else {
                        audio_inputs.push(format!("[0:a:{}][a{}]", track.index, i));
                    }
                }
            }
            
            if !audio_inputs.is_empty() {
                // Mix enabled tracks
                filter_complex = format!("{}{}amix=inputs={}[mixed]", 
                    audio_inputs.join(";"), 
                    if audio_inputs.len() > 1 { ";" } else { "" },
                    audio_inputs.len()
                );
                
                cmd.arg("-filter_complex").arg(&filter_complex);
                cmd.arg("-map").arg("0:v"); // Map video
                cmd.arg("-map").arg("[mixed]"); // Map mixed audio to track 1
                
                // Map original audio tracks
                for track in &clip.audio_tracks {
                    cmd.arg("-map").arg(format!("0:a:{}", track.index));
                }
            }
        }

        cmd.arg("-y") // Overwrite output file
            .arg(output_path);

        let output = cmd.output()?;
        
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("FFmpeg error: {}", error));
        }

        Ok(())
    }

    pub fn get_video_info(file_path: &Path) -> anyhow::Result<VideoInfo> {
        let output = Command::new("ffprobe")
            .arg("-v").arg("quiet")
            .arg("-print_format").arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(file_path)
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("ffprobe failed"));
        }

        let json_str = String::from_utf8(output.stdout)?;
        let info: serde_json::Value = serde_json::from_str(&json_str)?;
        
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
            
        let empty_vec = vec![];
        let streams = info["streams"].as_array().unwrap_or(&empty_vec);
        let mut audio_tracks = Vec::new();
        let mut audio_index = 0;
        
        for stream in streams.iter() {
            if stream["codec_type"].as_str() == Some("audio") {
                let default_name = format!("Audio Track {}", audio_index + 1);
                let track_name = stream["tags"]["title"]
                    .as_str()
                    .unwrap_or(&default_name);
                    
                audio_tracks.push(crate::core::AudioTrack {
                    index: audio_index,
                    enabled: true,
                    surround_mode: false,
                    name: track_name.to_string(),
                });
                audio_index += 1;
            }
        }

        Ok(VideoInfo {
            duration,
            audio_tracks,
        })
    }

    pub fn extract_thumbnail(file_path: &Path, timestamp: f64, output_path: &Path) -> anyhow::Result<()> {
        let output = Command::new("ffmpeg")
            .arg("-i").arg(file_path)
            .arg("-ss").arg(format!("{:.3}", timestamp))
            .arg("-vframes").arg("1")
            .arg("-f").arg("image2")
            .arg("-y")
            .arg(output_path)
            .output()?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Thumbnail extraction failed: {}", error));
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct VideoInfo {
    pub duration: f64,
    pub audio_tracks: Vec<crate::core::AudioTrack>,
}
