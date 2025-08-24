# MediaController Implementation Plan

## Objective
Create a robust MediaController that coordinates video and audio playback, preventing synchronization bugs through comprehensive testing and single-point-of-control architecture.

## Current Status
- ✅ **Phase 1 Complete**: Mock players with command tracking (22 tests)
- ✅ **Phase 2 Complete**: User state signaling with MediaControllerState enum (30 tests total)
- ✅ **Phase 2.5 Complete**: Thread safety analysis and GUI integration (33 tests total)
- ✅ **Phase 3 Complete**: Thread safety implementation with message-passing architecture (68 tests total)
- ✅ **Phase 4 Complete**: Edge cases and boundary condition testing (69 tests total) 
- ✅ Process lifecycle management and tracking implemented
- ✅ **MAJOR ACHIEVEMENT**: MediaController now Send + Sync through architectural solution
- ✅ **ThreadSafeAudioController**: Message passing for audio thread safety
- ✅ **ThreadSafeVideoController**: Message passing for video thread safety
- ✅ **CRITICAL BUG FIX**: Fixed NaN/Infinity handling in seek operations
- 🔄 **Next Phase**: Performance optimization, real implementation, or GUI integration
- ❌ GUI integration pending (would require significant refactoring of app.rs)

## Implementation Phases

### Phase 1: Mock-Based Command Verification Tests
**Goal**: Test that MediaController correctly coordinates commands to both players

**Tasks**:
1. Create `MockVideoPlayer` and `MockAudioPlayer` traits/structs
2. Add command recording to mocks (Vec of received commands)
3. **NEW: Add process tracking to mocks**:
   - Track spawned FFmpeg processes (process IDs)
   - Track killed processes 
   - Verify process cleanup on each operation
4. Write tests verifying command coordination:
   - `test_play_sends_coordinated_commands()`
   - `test_pause_sends_coordinated_commands()`
   - `test_seek_sends_same_timestamp_to_both()`
   - `test_seek_then_play_sends_correct_sequence()`
5. **NEW: Process lifecycle tests**:
   - `test_seek_spawns_new_processes_kills_old()`
   - `test_clip_change_kills_all_previous_processes()`
   - `test_pause_doesnt_kill_processes_unnecessarily()`

**Acceptance Criteria**:
- Mock players record all commands received
- **Mock players track all process spawns/kills**
- Tests verify both players get identical timestamps
- Tests verify correct command sequence/order
- **Tests verify no zombie processes remain after operations**
- Tests compile and pass (implementation not required yet)

### Phase 2: User State Signaling & Error Handling Tests
**Goal**: Define and test user-visible states during operations

**Tasks**:
1. Define MediaController states for user feedback:
   - `Loading`, `Playing`, `Paused`, `Seeking`, `Error(String)`, `Ready`
2. Add user state tracking to MediaController interface
3. Write user state tests:
   - `test_loading_state_during_video_load()`
   - `test_seeking_state_during_seek_operation()`
   - `test_error_state_when_video_fails_to_load()`
   - `test_ready_state_after_successful_load()`
4. Write error handling tests:
   - `test_video_play_fails_state_consistency()`
   - `test_audio_seek_fails_rollback_behavior()`
   - `test_partial_failure_recovery()`
   - `test_error_during_coordinated_operation()`

**Acceptance Criteria**:
- Clear user states defined for all operations
- User can distinguish between loading, ready, error states
- Error messages are meaningful and actionable
- State transitions are logical and tested
- Consistent error handling strategy across all operations

### Phase 2.5: Thread Safety & GUI Integration
**Goal**: Ensure MediaController works safely with egui

**Tasks**:
1. Define MediaController threading model:
   - Which methods can be called from GUI thread?
   - Which operations are blocking vs non-blocking?
   - How to handle background operations?
2. Write thread safety tests:
   - `test_gui_thread_safety()`
   - `test_concurrent_operations_handling()`
   - `test_non_blocking_operations()`
3. Design GUI integration pattern:
   - How does MediaController report state changes to GUI?
   - How does GUI handle loading/error states?

**Acceptance Criteria**:
- MediaController safe to call from egui update loop
- No blocking operations on GUI thread
- Clear pattern for GUI to display user states
- Thread safety verified through tests

### Phase 3: Edge Cases & Boundary Condition Tests
**Goal**: Test robustness under unusual conditions

**Tasks**:
1. Write edge case tests:
   - `test_rapid_multiple_seeks()`
   - `test_operations_without_video_loaded()`
   - `test_seek_beyond_boundaries()`
   - `test_concurrent_operations()`
   - `test_duplicate_operations()`

**Acceptance Criteria**:
- All edge cases handle gracefully without crashes
- Clear behavior specification for boundary conditions
- No undefined behavior scenarios

### Phase 4: Resource Management & Cleanup Tests
**Goal**: Ensure proper resource cleanup and process lifecycle management

**Tasks**:
1. Add process tracking to mock players
2. Write cleanup tests:
   - `test_drop_stops_all_players()`
   - `test_set_video_cleans_previous()`
   - `test_no_thread_leaks()`
   - `test_proper_shutdown_sequence()`
3. **NEW: Process lifecycle tests**:
   - `test_seek_kills_old_ffmpeg_processes()`
   - `test_clip_change_kills_all_old_processes()`
   - `test_only_expected_processes_alive()`
   - `test_process_cleanup_on_error()`
   - `test_no_zombie_processes_after_operations()`

**Acceptance Criteria**:
- All resources properly cleaned up
- No memory or thread leaks  
- Clean shutdown behavior specified
- **Process tracking**: Mock players track spawned/killed processes
- **Process assertions**: Tests verify exact processes alive at any time
- **Cleanup verification**: Old processes confirmed dead before new ones start

### Phase 5: MediaController Implementation
**Goal**: Implement actual functionality to pass all tests while preserving smooth video playback

**Tasks**:
1. Implement `play()` - coordinate both players
2. Implement `pause()` - coordinate both players
3. Implement `seek()` - coordinate both players with same timestamp
4. Implement `set_video()` - initialize both players
5. Implement `update_audio_tracks()` - update audio configuration
6. **CRITICAL: Preserve current video smoothness**:
   - Maintain hybrid approach (instant seeking + smooth playback)
   - Keep existing egui integration patterns
   - Preserve 30 FPS playback performance
7. Implement user state signaling as defined in Phase 2
8. Implement error handling as specified by tests
9. Implement resource cleanup and process management

**Acceptance Criteria**:
- All tests pass
- Real coordination between video and audio players
- **Video playback remains smooth and embedded in egui**
- **No regression in video quality or performance**
- User states properly reported to GUI
- Consistent with existing video/audio player interfaces
- No breaking changes to existing functionality

### Phase 6: GUI Integration
**Goal**: Replace current video/audio usage with MediaController

**Tasks**:
1. Update `ClipHelperApp` to use `MediaController` instead of direct players
2. Remove direct access to `EmbeddedVideoPlayer` and `SynchronizedAudioPlayer`
3. Update all play/pause/seek calls to go through MediaController
4. Implement user state display in GUI (loading indicators, error messages)
5. **Preserve current GUI experience**:
   - Video remains embedded in timeline
   - No change to user interaction patterns
   - Maintain current responsiveness

**Acceptance Criteria**:
- GUI only interacts with MediaController
- All existing functionality works
- Audio and video stay synchronized
- **User experience unchanged or improved**
- Loading states and errors properly displayed
- No regressions in user experience

### Phase 7: Real Integration Testing
**Goal**: Test with actual video/audio files

**Tasks**:
1. Create minimal test video files
2. Write integration tests with real files:
   - `test_real_video_audio_sync()`
   - `test_real_seeking_accuracy()`
   - `test_real_track_switching()`

**Acceptance Criteria**:
- Tests use actual video files (small test files)
- Verify real synchronization between video and audio
- End-to-end validation of MediaController

### Phase 8: Cleanup & Removal of Old Components
**Goal**: Remove unused code and consolidate architecture

**Tasks**:
1. Remove or deprecate old components no longer needed:
   - Direct `EmbeddedVideoPlayer` usage outside MediaController
   - Direct `SynchronizedAudioPlayer` usage outside MediaController
   - Old synchronization helper code
   - Redundant audio/video coordination logic
2. Clean up imports and module structure:
   - Remove unused exports from `video::mod.rs`
   - Update documentation to reflect new architecture
   - Remove dead code and commented-out sections
3. Consolidate configuration:
   - Ensure all video/audio configuration goes through MediaController
   - Remove duplicate configuration paths
4. Update error handling:
   - Remove old error handling patterns
   - Consolidate error types and reporting

**Acceptance Criteria**:
- Codebase only has one way to control video/audio (MediaController)
- No dead code or unused components remain
- Clean module structure with clear responsibilities
- All tests still pass after cleanup
- Documentation reflects current architecture
- Reduced code complexity and maintenance burden

## Design Principles

### Single Point of Control
- **ONLY** MediaController has public play/pause/seek methods
- Video and audio players are internal implementation details
- Impossible to call one player without the other

### Command Coordination
- All operations send coordinated commands to both players
- Same timestamps passed to both players
- Consistent error handling across both players

### State Consistency
- MediaController maintains authoritative state
- State remains consistent even when operations fail
- Clear rollback behavior when partial failures occur

### **Process Lifecycle Management**
- **Process tracking**: All spawned FFmpeg processes must be tracked
- **Cleanup verification**: Old processes confirmed killed before spawning new ones
- **No zombies**: Zero tolerance for background processes after operations complete
- **Clip isolation**: Switching clips must kill ALL processes from previous clip
- **Seek cleanup**: Each seek operation must kill previous video/audio processes

### Comprehensive Testing
- Every public method has comprehensive tests
- Error conditions explicitly tested
- Edge cases and boundary conditions covered
- Real integration testing with actual files
- **Process lifecycle explicitly tested and verified**

## Test Strategy

### Mock-Based Testing (Phases 1-4)
- Mock players record all commands received
- **Mock players track all process spawns/kills**
- Tests verify command coordination without real video/audio
- **Tests verify process cleanup without real FFmpeg**
- Fast execution, deterministic results
- Focus on coordination logic and process lifecycle

### Integration Testing (Phase 7)
- Real video/audio players with test files
- Verify actual synchronization behavior
- Catch issues that mocks might miss
- Slower but more realistic

### Error Injection Testing
- Mock players can simulate failures
- Test all failure scenarios
- Verify error handling and state consistency
- Ensure graceful degradation

## Implementation Rules

### Never Break These Rules:
1. **NO direct access** to video/audio players from outside MediaController
2. **ALWAYS coordinate** - never call one player without the other
3. **SAME timestamps** - both players must get identical seek positions
4. **Consistent state** - MediaController state must reflect reality
5. **Test first** - write tests before implementation
6. **Handle errors** - all failure scenarios must be explicitly handled
7. **KILL old processes** - before spawning new ones, confirm old ones are dead
8. **TRACK all processes** - every spawned process must be tracked and accounted for
9. **NO zombie processes** - zero background processes after operations complete

### Code Review Checklist:
- [ ] Does this operation coordinate both video and audio?
- [ ] Do both players get the same timestamp?
- [ ] Is error handling consistent?
- [ ] Are there tests covering this scenario?
- [ ] Is MediaController state updated correctly?
- [ ] Are resources properly cleaned up?
- [ ] **Are old processes killed before spawning new ones?**
- [ ] **Are all spawned processes tracked?**
- [ ] **Are there assertions verifying no zombie processes?**

## Critical Process Lifecycle Scenarios

### Scenario 1: Clip Change
**Current State**: Playing clip A with video + audio FFmpeg processes
**Action**: User selects clip B  
**Required Behavior**:
1. Kill ALL processes for clip A (video + audio)
2. Confirm processes are dead (wait for exit)
3. Load clip B and spawn new processes
4. **Test assertion**: Only clip B processes alive, zero clip A processes

### Scenario 2: Seeking During Playback  
**Current State**: Playing with active video stream process
**Action**: User seeks to new timestamp
**Required Behavior**:
1. Kill current video stream process
2. Confirm process is dead
3. Spawn new video stream from seek position
4. **Test assertion**: Only new stream process alive, old stream dead

### Scenario 3: Audio Track Changes
**Current State**: Playing with audio process for tracks [0,1]
**Action**: User enables track 2, disables track 1  
**Required Behavior**:
1. Kill current audio process
2. Confirm process is dead
3. Spawn new audio process with tracks [0,2]
4. **Test assertion**: Only new audio process alive, old audio dead

### Scenario 4: Pause/Resume
**Current State**: Playing with active processes
**Action**: User pauses, then resumes
**Required Behavior**:
- **Option A**: Keep processes alive during pause (current embedded_player approach)
- **Option B**: Kill processes on pause, respawn on resume
- **Test assertion**: Verify chosen behavior consistently applied

### Scenario 5: Error During Process Spawn
**Current State**: Attempt to spawn new process fails
**Action**: FFmpeg command fails to start
**Required Behavior**:
1. Ensure old processes were still killed (don't leave them running)
2. MediaController in consistent error state
3. **Test assertion**: No processes alive if spawn failed

## Mock Player Process Tracking

```rust
#[derive(Debug, Clone)]
struct ProcessRecord {
    id: u32,
    command: String,
    status: ProcessStatus,
    spawned_at: Instant,
    killed_at: Option<Instant>,
}

#[derive(Debug, Clone)]
enum ProcessStatus {
    Spawned,
    Running, 
    Killed,
    Died, // Process ended on its own
}

struct MockVideoPlayer {
    commands: Vec<(String, f64)>,
    processes: Vec<ProcessRecord>, // Track all processes
    active_processes: HashSet<u32>, // Currently alive processes
}
```

## Success Metrics
- All tests in phase pass
- Code compiles without warnings
- No regressions in existing functionality

### Final Success:
- Zero synchronization bugs between video and audio
- All operations consistently coordinate both players
- Robust error handling and edge case coverage
- Clean, maintainable architecture
- Comprehensive test coverage

## Risk Mitigation

### Risk: Breaking existing functionality
**Mitigation**: Implement in phases, test thoroughly at each step

### Risk: Performance regression
**Mitigation**: Performance tests in Phase 3, benchmark against current implementation

### Risk: Complex error states
**Mitigation**: Explicit error handling tests in Phase 2, clear failure behavior specification

### Risk: Thread safety issues
**Mitigation**: Concurrency tests in Phase 3, careful review of thread interactions

## Timeline Estimate

- **Phase 1**: Mock tests with process tracking - 1 day
- **Phase 2**: User state signaling & error handling tests - 1 day  
- **Phase 2.5**: Thread safety & GUI integration - 0.5 days
- **Phase 3**: Edge case tests - 1 day
- **Phase 4**: Resource cleanup tests - 0.5 days
- **Phase 5**: Implementation (preserving video quality) - 2 days
- **Phase 6**: GUI integration with state display - 1 day
- **Phase 7**: Real integration tests - 0.5 days
- **Phase 8**: Cleanup & removal of old components - 0.5 days

**Total**: ~8 days

## Next Step
Start with Phase 1: Create mock players and command verification tests.
