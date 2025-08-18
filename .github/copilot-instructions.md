# ClipHelper AI Assistant Instructions

ClipHelper is a Rust application for trimming OBS replay buffer clips with global hotkeys, timeline editing, and FFmpeg integration. It helps streamline the process of capturing and editing gaming clips from OBS replay buffer with precise timing controls.

## Core Functionality

### Global Hotkey System
- **Ctrl+1**: Capture 15-second clip
- **Ctrl+2**: Capture 30-second clip  
- **Ctrl+3**: Capture 1-minute clip
- **Ctrl+4**: Capture 2-minute clip
- **Ctrl+5**: Capture 5-minute clip

### Smart File Matching
- Monitors OBS replay directory for "Replay YYYY-MM-DD HH-MM-SS.mkv" files
- Matches hotkey timestamps to replay files within 10-second tolerance window
- Automatically identifies the closest matching replay file for clip creation

### Timeline Editor Interface
- Visual timeline with scrubbing controls for precise navigation
- Draggable trim handles for start/end point adjustment
- Video preview with playback controls (play/pause, seek, frame stepping)
- Audio waveform visualization for multiple tracks
- Time-based navigation: skip 3s/5s/10s forward/backward, go to start/end

### Advanced Audio Management
- Multiple audio track support with individual enable/disable
- Surround sound channel mapping (FL|FR for spatial audio): Maps selected tracks to front-left/front-right channels so they can be disabled separately while still being audible in the mixed output
- Track mixing: Track 1 = mixed output, Track 2+ = original tracks preserved
- Visual audio controls with surround mode checkboxes

### Precise Trim Controls
- Start/end time adjustment buttons: ±1s, ±5s increments
- Timeline scrubbing with frame-accurate positioning
- Real-time preview of trim points
- Custom clip naming with format: "Original Name - Custom Name.mkv"

### File Management System
- **Apply**: Trims clip and saves to "trimmed/" directory within the watched OBS replay directory
- **Delete**: Moves original file to "deleted/" directory within the watched OBS replay directory
- Original replay files never modified
- Organized output structure with separate folders for different operations within the monitored directory

## Architecture Overview

- **Core (`src/core/`)**: Data structures (`Clip`, `AppConfig`), file monitoring, timestamp matching
- **GUI (`src/gui/`)**: egui-based interface with modular components (timeline, controls, clip list)
- **Hotkeys (`src/hotkeys/`)**: Global Windows hotkey system for background clip capture
- **Video (`src/video/`)**: FFmpeg command-line integration for trimming, preview, waveforms

## Key Design Patterns

### Timestamp Matching System
Clips match OBS replay files within 10-second windows using filename parsing:
```rust
// Extract from "Replay 2025-08-17 21-52-01.mkv" format
pub fn extract_timestamp_from_filename(file: &PathBuf) -> anyhow::Result<DateTime<Utc>>
```

### Audio Track Architecture
Complex audio handling with surround sound and track mixing:
- Track 1 = mixed output from enabled tracks
- Track 2+ = original tracks preserved
- `surround_mode` maps FL|FR channels for spatial audio: This allows tracks to be disabled separately while still being audible in the mixed output

### FFmpeg Integration
Command-line FFmpeg for maximum compatibility. Key patterns:
- Use `-c:v copy` for fast video copying without re-encoding
- Generate complex filter graphs for audio track mixing
- Use `-y` flag to overwrite outputs only when shift-clicking (normal clicks should prompt for confirmation)

## Development Workflows

### Building & Running
```bash
cargo build --release     # Production build
RUST_LOG=debug cargo run  # Debug with logging
```

### FFmpeg Dependencies
- Requires `ffmpeg` and `ffprobe` in PATH
- Configured via `AppConfig.ffmpeg_path` or system PATH
- Commands built dynamically in `video::processor::VideoProcessor`

### Configuration System
JSON config stored in `%APPDATA%\clip-helper\config.json`:
```json
{
  "obs_replay_directory": "path/to/replays",
  "output_directory": "path/to/output",
  "deleted_directory": "path/to/output/deleted",
  "trimmed_directory": "path/to/output/trimmed",
  "last_watched_directory": "path/to/last/watched"
}
```
- Last watched directory is restored on startup; if none exists, no directory is monitored until user selects one
- Trimmed and deleted directories are created within the watched directory

## Project-Specific Conventions

### Error Handling
- Use `anyhow::Result` for application errors
- `thiserror` for structured error types
- FFmpeg errors parsed from stderr output

### Async Architecture
- Tokio runtime for file monitoring and background tasks
- `broadcast` channels for hotkey event communication
- GUI runs on main thread, background tasks on Tokio

### File Organization
- Original files never modified
- Deleted clips moved to `deleted/` subfolder within watched directory
- Trimmed clips saved to `trimmed/` subfolder within watched directory
- Custom naming with format: "Original Name - Custom Name.mkv"

### Testing Strategy
- Unit tests for core data structures, file operations, and timestamp parsing
- Integration tests for FFmpeg processing and file management workflows
- Mock file systems for testing file monitoring and organization
- All major functionality should be testable without requiring actual video files or global hotkeys

## Integration Points

### OBS Integration
- Monitors replay directory for files matching "Replay YYYY-MM-DD HH-MM-SS.mkv"
- Hotkey timestamps matched against file creation times within 10-second tolerance
- Supports OBS replay buffer workflow: record continuously, save on hotkey press
- Handles multiple replay files and automatic cleanup of old recordings

### Windows-Specific Features
- Global hotkeys work even when application is not focused
- Uses Win32 APIs via `global-hotkey` crate for system-wide shortcuts
- Configuration paths follow Windows standards via `dirs` crate
- Currently Windows-only due to global hotkey implementation requirements

### Video Processing Pipeline
- FFmpeg command-line integration for maximum format compatibility
- Supports complex audio mixing with multiple input tracks
- Maintains video quality with copy codec (no re-encoding)
- Handles various OBS output formats (mkv, mp4, etc.)

## User Interface Components

### Main Application Layout
- **Left Sidebar**: Scrollable clip list with thumbnails and metadata
- **Central Panel**: Timeline editor with video preview and waveform display
- **Bottom Panel**: Playback controls, trim adjustments, and action buttons
- **Top Menu**: Settings, file operations, and application controls

### Timeline Features
- Horizontal timeline with time markers and current position indicator
- Draggable trim handles at start and end positions
- Mouse scrubbing for precise frame-by-frame navigation
- Audio waveform overlay for visual audio editing
- Zoom controls for detailed editing of short clips

### Control Buttons Specification
- **Playback**: Play/pause, go to start, go to last 5 seconds
- **Navigation**: Skip ±3s, ±5s, ±10s with keyboard shortcuts
- **Trim Adjustment**: Start/end time ±1s, ±5s buttons
- **File Operations**: Apply (trim & save), Delete (move to deleted folder)
- **Audio**: Individual track enable/disable, surround mode toggles

## Common Development Tasks

### Adding New Hotkeys
1. Add variant to `HotkeyId` enum in `hotkeys/events.rs`
2. Register in `HotkeyManager::new()` with appropriate key code  
3. Handle events in GUI `ClipHelperApp::update()`
4. Update documentation and user interface labels

### Video Processing Changes
- Modify FFmpeg commands in `video::processor::VideoProcessor`
- Test with various video formats and track configurations
- Always validate FFmpeg output status and stderr
- Consider performance impact of complex filter graphs

### GUI Component Development
- Use egui immediate mode patterns for responsive UI
- Store persistent state in `ClipHelperApp` struct
- Separate concerns: timeline, controls, clip list as independent modules
- Follow egui best practices for layout and interaction

### Timeline & Preview Features
- Timeline scrubbing via mouse position mapping to time
- Draggable trim handles as interactive UI elements
- Real-time waveform generation using `hound` crate for audio analysis
- Video frame extraction for preview thumbnails and scrubbing

### Audio System Extensions
- Multi-track audio visualization in timeline
- Dynamic audio mixing based on user selections
- Surround sound processing with channel mapping
- Real-time audio level monitoring during playback

## Implementation Status & Next Steps

### Completed Foundation
- Core data structures and configuration system
- Basic GUI framework with egui integration
- FFmpeg command generation for video processing
- Global hotkey registration system
- File monitoring and timestamp matching logic

### In Development
- Timeline widget with scrubbing capabilities
- Real-time video preview integration
- Audio waveform visualization
- Hotkey event processing (API compatibility fixes needed)
- File monitoring for new OBS replay files

### Future Enhancements
- Batch processing for multiple clips
- Export presets and quality settings
- Keyboard shortcuts for all timeline operations
- Plugin system for custom processing filters
- Cross-platform support (Linux, macOS)
