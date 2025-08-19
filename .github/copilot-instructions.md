# ClipHelper AI Assistant Instructions

ClipHelper is a Rust application for trimming OBS replay buffer clips with global hotkeys, timeline editing, and FFmpeg integration. This document provides development guidance for AI assistants working on the codebase.

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

### Session Grouping Algorithm
Clips are automatically grouped into recording sessions:
- Sessions created when gap between clips > 1 hour
- Sessions displayed in descending chronological order (newest first)
- Format: "YYYY-MM-DD - session HH:MM - HH:MM"

### Audio Track Architecture
Complex audio handling with surround sound and track mixing:
- Track 1 = mixed output from enabled tracks
- Track 2+ = original tracks preserved
- `surround_mode` maps FL|FR channels for spatial audio

### FFmpeg Integration
Command-line FFmpeg for maximum compatibility. Key patterns:
- Use `-c:v copy` for fast video copying without re-encoding
- Generate complex filter graphs for audio track mixing
- Use `-y` flag to overwrite outputs only when shift-clicking

### Non-Blocking Architecture
- **Immediate startup**: UI loads instantly with clip list from file scan
- **Background video info**: Video duration and audio track info loaded lazily
- **Real-time updates**: New files appear immediately via file system monitoring
- **Performance optimization**: Startup limited to 50 most recent files, manual refresh loads all

## Development Workflows

### Building & Testing
```bash
cargo check               # Check compilation (preferred for development)
cargo test                # Run test suite
cargo build --release     # Production build (only when needed)
RUST_LOG=debug cargo run  # Debug with logging (Windows: $env:RUST_LOG="debug"; cargo run)
```

### Configuration System
JSON config stored in `%APPDATA%\clip-helper\config.json`:
```json
{
  "obs_replay_directory": "path/to/replays",
  "last_watched_directory": "path/to/last/watched"
}
```

## Project-Specific Conventions

### Error Handling
- Use `anyhow::Result` for application errors
- `thiserror` for structured error types
- FFmpeg errors parsed from stderr output

### Async Architecture
- Tokio runtime for file monitoring and background tasks
- `broadcast` channels for hotkey event communication and file detection
- GUI runs on main thread, background tasks on Tokio
- Real-time file monitoring via `notify` crate for immediate updates

### File Organization
- Original files never modified
- Deleted clips moved to `deleted/` subfolder within watched directory
- Trimmed clips saved to `trimmed/` subfolder within watched directory
- Custom naming with format: "Original Name - Custom Name.mkv"

### Testing Strategy
- Unit tests for core data structures, file operations, and timestamp parsing
- Integration tests for FFmpeg processing and file management workflows
- Mock file systems for testing file monitoring and organization
- Use `cargo test` for running tests, `cargo check` for compilation validation

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
- **Avoid blocking operations in UI thread**: Use lazy loading and background processing

### Performance Guidelines
- **Startup optimization**: Load UI immediately, defer heavy operations
- **Lazy loading**: Video metadata only loaded when needed (selection or display)
- **Session-based organization**: Efficient grouping without loading all video data
- **File monitoring**: Event-driven updates instead of polling

## Integration Points

### OBS Integration
- Monitors replay directory for files matching "Replay YYYY-MM-DD HH-MM-SS.mkv"
- Hotkey timestamps matched against file creation times within 10-second tolerance
- **Immediate detection**: New files appear in UI instantly when OBS creates them

### Windows-Specific Features
- Global hotkeys work even when application is not focused
- Uses Win32 APIs via `global-hotkey` crate for system-wide shortcuts
- Currently Windows-only due to global hotkey implementation requirements

### Video Processing Pipeline
- FFmpeg command-line integration for maximum format compatibility
- Supports complex audio mixing with multiple input tracks
- **Lazy loading**: Video information (duration, audio tracks) loaded on-demand
- **Continuous monitoring**: Files being written by OBS are checked every 2 seconds for completion
- **Progressive validation**: Invalid files (<1s duration) are rechecked until they become valid

## Implementation Guidelines

### Video Info Lifecycle Management
- **Initial state**: New clips have no video info (`video_length_seconds: None`)
- **Gray out phase**: Display clips as grayed out while `needs_video_info_update()` returns true
- **Periodic updates**: Check grayed-out files every 2 seconds via `update_pending_video_info()`
- **Validation criteria**: Files with duration >= 1.0 seconds are considered valid
- **Error handling**: Files that can't be read are marked with 0.0 duration and retried later

### Hotkey Target Duration Assignment
- **Immediate application**: Hotkeys first try to find existing clips matching the timestamp
- **Deferred application**: If no existing clip found, store request for future file matching
- **Resilient matching**: Target duration applies even to grayed-out (invalid) clips
- **Extended window**: Pending requests remain active for 30 seconds to catch delayed file creation

### UI Responsiveness
- **Non-blocking operations**: No FFmpeg calls during UI rendering
- **Immediate feedback**: "Loading..." states shown while processing
- **Background updates**: Video info populated after UI interaction

### Memory Management
- **Lazy loading**: Video metadata only loaded when needed
- **Limited initial scan**: Only recent files loaded at startup
- **Manual refresh option**: Full scan available on demand

### Code Quality
- Follow Rust best practices and idioms
- Use appropriate error handling with `anyhow` and `thiserror`
- Write unit tests for core functionality
- Document complex algorithms and design decisions

## File State Management
- **File validation**: Check for valid video duration (>1s) before displaying as available
- **Gray out invalid files**: Files being written or with invalid duration should be visually disabled
- **Target length handling**: Only set target length when explicitly specified via hotkeys
- **Default state**: New clips should have unspecified target length until hotkey assigns one
- **Continuous updates**: Files being written by OBS are periodically checked and updated when complete
- **Hotkey resilience**: Hotkeys work on grayed-out files and apply to files created before detection
