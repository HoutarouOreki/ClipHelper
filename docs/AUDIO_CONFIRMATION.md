# Audio Confirmation Feature

ClipHelper now includes an audio confirmation system that plays a sound when new OBS replay clips are detected.

## Features

### Duration-Specific Confirmation Sounds
- **15s clips**: 1 short beep (0.1s) at 1000Hz
- **30s clips**: 2 short beeps with 0.05s pause between them
- **1m clips**: 3 short beeps with 0.05s pauses
- **2m clips**: 4 short beeps with 0.05s pauses  
- **5m clips**: 5 short beeps with 0.05s pauses
- **Unmatched clips**: Single 0.5s low frequency (400Hz) sound when no duration request matches

### Configurable Audio Output
- **Device Selection**: Choose specific audio output device or use system default
- **Volume Control**: Adjustable volume from 0% to 100%
- **Device Refresh**: Refresh available audio devices without restarting

### Sound File Management
- **Custom Sounds**: Browse and select any WAV audio file for general clip detection
- **Auto-Generated Duration Sounds**: Automatically creates duration-specific beep patterns
- **Default Sound Generator**: Automatically generates a simple beep sound for testing
- **File Validation**: Checks for sound file existence and logs errors appropriately

### Settings Integration
- **Settings Dialog**: Accessible via File > Settings menu
- **Real-time Testing**: Test button to preview confirmation sound
- **Persistent Configuration**: Settings saved to `%APPDATA%\clip-helper\config.json`

## Usage

### Sound Types

**General Clip Detection**: Plays the configured custom sound file when any new replay file is detected (only if no duration-specific sound is played)

**Duration Matching**: When a clip gets matched with a hotkey duration request:
- 15s clips: Single short beep
- 30s clips: Two short beeps  
- 1m clips: Three short beeps
- 2m clips: Four short beeps
- 5m clips: Five short beeps

**Unmatched Clips**: When a clip is detected but no hotkey was pressed within 10 seconds, plays a longer low-frequency sound to indicate the clip needs manual attention

### Enabling Audio Confirmation

1. Open ClipHelper
2. Go to File > Settings
3. Check "Enable confirmation sound when clips are detected"
4. Configure your preferred settings:
   - Choose audio output device (optional - defaults to system default)
   - Adjust volume level
   - Select or generate a sound file

### Setting Up Sound File

**Option 1: Generate Default Sound**
1. In Settings dialog, click "Generate Default" 
2. This creates a simple 800Hz beep in `%APPDATA%\clip-helper\default_confirmation.wav`

**Option 2: Use Custom Sound File**
1. Click "Browse..." to select any WAV audio file
2. File will be validated when playing

### Testing

- Use the "Test Sound" button in settings to preview your configuration
- Check the status bar for confirmation when sounds play successfully

## Technical Details

### Audio System
- **Backend**: Uses `cpal` for device enumeration and `rodio` for playback
- **Format Support**: WAV files via `hound` decoder and generator
- **Auto-Generation**: Creates duration-specific beep patterns automatically
- **Non-blocking**: Sound playback doesn't block the UI
- **Error Handling**: Comprehensive logging for troubleshooting

### Configuration Format
```json
{
  "audio_confirmation": {
    "enabled": false,
    "sound_file_path": "C:\\Users\\...\\default_confirmation.wav",
    "output_device_name": "Speakers (Realtek Audio)",
    "volume": 0.5
  }
}
```

### Integration Points
- **Clip Detection**: Triggered when `FileMonitor` detects new replay files
- **Duration Matching**: Plays specific patterns when clips get matched with hotkey requests
- **Display-time Matching**: Works with the persistence-based duration system
- **Unmatched Detection**: Plays warning sound for clips without duration requests
- **Error Recovery**: Falls back gracefully if audio system unavailable

## Troubleshooting

### No Sound Playing
1. Check if audio confirmation is enabled in settings
2. Verify sound file exists and is valid WAV format
3. Try refreshing audio devices
4. Check application logs for audio errors
5. Test with generated default sound

### Audio Device Issues
1. Use "Refresh" button to update device list
2. Try selecting "(Default)" device option
3. Check Windows audio settings for device availability

### Performance
- Audio confirmation is lightweight and non-blocking
- Failed playback attempts are logged but don't affect core functionality
- Sound generation creates small WAV files (~8KB for 200ms beep)

## Logging

Audio confirmation events are logged with appropriate levels:
- `INFO`: Successful operations, device detection
- `WARN`: Fallback scenarios, missing files
- `ERROR`: System failures, invalid configurations
- `DEBUG`: Detailed operation tracing

Check logs with: `RUST_LOG=debug cargo run`
