⚠️ VIBE CODING EXPERIMENT

# Clip Helper - OBS Replay Buffer Trimmer

A Rust application designed to help streamline the process of trimming and organizing clips from OBS's replay buffer feature.

## Features

### Core Functionality
- **Global Hotkeys**: Capture clips with Ctrl+Numpad1-5 for different durations (15s, 30s, 1m, 2m, 5m)
- **Smart Duration Matching**: Persistent duration request system allows multiple duration changes within 10-second window
- **Latest Request Wins**: Most recent hotkey press always takes precedence for duration assignment
- **Real-time Auto-detection**: New replay files automatically appear in clip list immediately
- **Session Grouping**: Clips organized by recording sessions (gaps > 1 hour create new sessions)
- **Timeline Editor**: Visual timeline with scrubbing controls for precise trimming
- **Video Preview**: Built-in video player with playback controls
- **Audio Track Management**: Enable/disable tracks and configure surround sound options
- **Non-blocking Startup**: UI appears instantly, file scanning and video info load in background

### Enhanced User Interface
- **300px wide sidebar** with full-width scrollable clip list
- **Session display format**: "2025-08-19 - session 14:56 - 17:11" (newest sessions first)
- **Rich duration display**: 
  - Video length: "2m 40s" (actual file duration)
  - Target length: "30s" (from hotkey trigger)
- **Auto-refresh**: Manual refresh button and automatic detection of new files

### Hotkey Mappings
- `Ctrl+Numpad1` = 15 second clip
- `Ctrl+Numpad2` = 30 second clip  
- `Ctrl+Numpad3` = 1 minute clip
- `Ctrl+Numpad4` = 2 minute clip
- `Ctrl+Numpad5` = 5 minute clip

### Controls
- **Playback**: Play/pause, seek to start, seek to last 5 seconds
- **Navigation**: Skip forward/backward by 3s, 5s, 10s
- **Trimming**: Adjust start/end times by 1s or 5s increments
- **File Management**: Delete (moves to "deleted" folder), Apply trim (saves to "trimmed" folder)

### Audio Confirmation
- **Clip Detection Sounds**: Optional audio notification when new clips are detected
- **Duration-Specific Sounds**: Optional beep patterns when clips are marked with target durations (1-5 beeps for 15s-5m)
- **Unmatched Hotkey Sounds**: Low-frequency sound when hotkey pressed but no clips available to match
- **Configurable Output Device**: Choose specific audio device or use system default
- **Volume Control**: Adjustable volume levels (0-100%)
- **Custom Sound Files**: Use any WAV file or generate default beep sound
- **Editable File Paths**: Manually edit the sound file path or browse with built-in/system dialogs
- **File Browser Options**: Choose between built-in file browser or system file dialog
- **Settings Integration**: Configure via File > Settings menu

### Audio Features
- Visual audio waveforms for each track
- Enable/disable individual audio tracks
- Surround left/right channel mapping option: Maps selected tracks to FL|FR channels so they can be disabled separately while still being audible in the mixed output
- Mixed output: Track 1 = mixed audio, Track 2+ = original tracks preserved

## Requirements

### System Dependencies
- **FFmpeg**: Required for video/audio processing
  - Download from [ffmpeg.org](https://ffmpeg.org/download.html)
  - Ensure `ffmpeg` and `ffprobe` are in your system PATH
- **Rust**: Version 1.70+ required
- **Windows**: Currently Windows-only due to global hotkey implementation

### OBS Setup
- Configure OBS replay buffer to save files with format: "Replay YYYY-MM-DD HH-MM-SS.mkv"
- Set up a dedicated directory for replay buffer files

## Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd ClipHelper
```

2. Install Rust dependencies:
```bash
cargo build --release
```

3. Install FFmpeg:
   - Download FFmpeg from the official website
   - Extract to a folder (e.g., `C:\ffmpeg`)
   - Add the `bin` directory to your system PATH

## Usage

1. **Initial Setup**:
   - Run the application: `cargo run --release`
   - Configure the OBS replay directory in Settings
   - Set output directories for trimmed and deleted clips
   - Last watched directory is automatically restored on startup; if none exists, no directory is monitored until user selects one

2. **Capturing Clips**:
   - While OBS is recording with replay buffer enabled
   - Press `Ctrl+1` through `Ctrl+5` when something interesting happens
   - Duration requests are saved with timestamps for later matching
   - Multiple hotkey presses within 10 seconds will update the same clip (latest wins)
   - The application will automatically find matching replay files and apply durations

3. **Editing Clips**:
   - Select a clip from the list on the left
   - Use the timeline to scrub through the video
   - Adjust start/end times with the trim controls
   - Configure audio tracks as needed
   - Click "Apply" to save the trimmed clip

4. **File Organization**:
   - Original files remain untouched
   - Deleted clips move to the "deleted" subfolder within the watched directory
   - Trimmed clips save to the "trimmed" subfolder within the watched directory
   - Custom names are appended: "Original Name - Custom Name.mkv"
   - Normal clicks prompt for confirmation if file exists; shift+click overwrites automatically

## Project Structure

```
src/
├── main.rs              # Application entry point
├── core/                # Core data structures and logic
│   ├── mod.rs
│   ├── clip.rs          # Clip data structure and metadata
│   ├── config.rs        # Application configuration
│   └── file_monitor.rs  # File system monitoring
├── gui/                 # User interface components
│   ├── mod.rs
│   ├── app.rs           # Main application window
│   ├── clip_list.rs     # Clip list sidebar
│   ├── timeline.rs      # Timeline editor widget
│   └── controls.rs      # Playback controls
├── hotkeys/             # Global hotkey management
│   ├── mod.rs
│   ├── manager.rs       # Hotkey registration and handling
│   └── events.rs        # Hotkey event definitions
└── video/               # Video processing
    ├── mod.rs
    ├── processor.rs     # FFmpeg integration for trimming
    ├── preview.rs       # Video preview functionality
    └── waveform.rs      # Audio waveform generation
```

## Implementation Status

### ✅ Completed
- Project structure and dependencies
- Core data structures (Clip, Config)
- Global hotkey system foundation
- FFmpeg integration for video processing
- Basic GUI framework setup
- Audio track management structure

### 🚧 In Progress
- GUI timeline component with scrubbing
- Video preview integration
- Waveform visualization
- File monitoring for new replay files

### 📋 TODO
- Settings dialog for configuration
- Thumbnail generation for clip list
- Drag-and-drop timeline handles
- Real-time video preview
- Batch processing operations
- Keyboard shortcuts for timeline navigation
- Export presets and quality settings
- Plugin system for custom processing

## Development

### Building
```bash
# ## Debug

### Logging
```bash
# Enable debug logging to see detailed timestamp matching
RUST_LOG=debug cargo run

# Info level for general application flow
RUST_LOG=info cargo run

# See hotkey processing and file matching
RUST_LOG=clip_helper=debug cargo run
```

### Timezone Handling
- All timestamps use **local time** (not UTC) for accurate matching
- OBS replay file timestamps: Parsed as local time from filename
- Hotkey timestamps: Recorded in local time when pressed
- Matching window: 10-second tolerance accounts for timing differences
- **Note**: No timezone conversion is performed - both hotkeys and files should be in the same local timezone build
cargo build

# Release build
cargo build --release

# Run with logging
RUST_LOG=debug cargo run
```

### Testing
```bash
# Run tests
cargo test

# Run with specific test
cargo test test_name
```

### Testing Philosophy
- **Unit Tests**: Core data structures, file operations, timestamp parsing
- **Integration Tests**: FFmpeg processing, file management workflows  
- **Mock Systems**: File monitoring and organization testing without actual files
- **Testable Design**: All major functionality designed to be testable without requiring actual video files or global hotkeys

### Architecture Notes

- **GUI Framework**: Uses egui for cross-platform native UI
- **Video Processing**: FFmpeg via command-line interface for maximum compatibility
- **Global Hotkeys**: Windows-specific implementation using Win32 APIs
- **Async Operations**: Tokio runtime for file monitoring and background tasks
- **Error Handling**: Comprehensive error handling with anyhow and thiserror

## Configuration

The application stores configuration in:
- Windows: `%APPDATA%\clip-helper\config.json`
- Duration requests: `%APPDATA%\clip-helper\duration_requests.json`  
- Clip persistence: `%APPDATA%\clip-helper\clips.json`

Example configuration:
```json
{
  "obs_replay_directory": "C:\\Users\\Username\\Videos\\OBS Replays",
  "output_directory": "C:\\Users\\Username\\Videos\\Clips",
  "deleted_directory": "C:\\Users\\Username\\Videos\\Clips\\deleted",
  "trimmed_directory": "C:\\Users\\Username\\Videos\\Clips\\trimmed",
  "last_watched_directory": "C:\\Users\\Username\\Videos\\OBS Replays",
  "ffmpeg_path": null
}
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## License

[Add your chosen license here]

## Troubleshooting

### Common Issues

1. **FFmpeg not found**:
   - Ensure FFmpeg is installed and in PATH
   - Or set the full path in configuration

2. **Global hotkeys not working**:
   - Run as administrator if needed
   - Check for conflicting hotkey assignments

3. **Video files not found**:
   - Verify OBS replay buffer directory path
   - Check file naming format matches expected pattern

4. **Performance issues**:
   - Large video files may take time to process
   - Consider using proxy files for preview
