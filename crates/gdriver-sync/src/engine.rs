//! Sync engine state machine and main loop types.
//!
//! This module defines the core state machine shared between the daemon's
//! sync loop and the UI layer.  It is deliberately free of I/O so it can
//! be tested without database / network fixtures.

/// Internal state of the sync engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEngineState {
    /// No work to do; waiting for local changes or the next remote poll.
    Idle,
    /// Scanning the local filesystem or Drive for changes to build a task list.
    Scanning,
    /// Actively uploading, downloading, or reconciling files.
    Syncing,
    /// Sync has been suspended by the user; no tasks are processed.
    Paused,
}

impl SyncEngineState {
    /// Map the internal engine state to the user-visible [`SyncStatus`].
    pub fn to_sync_status(self) -> gdriver_ipc::SyncStatus {
        match self {
            Self::Idle => gdriver_ipc::SyncStatus::UpToDate,
            Self::Scanning | Self::Syncing => gdriver_ipc::SyncStatus::Syncing,
            Self::Paused => gdriver_ipc::SyncStatus::Paused,
        }
    }
}

// ─── Commands ─────────────────────────────────────────────────────────────────

/// Commands sent from IPC handlers to the sync engine.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncCommand {
    /// Suspend all sync processing.
    Pause,
    /// Resume processing after a pause.
    Resume,
    /// Switch the sync mode (Stream ↔ Mirror).
    SwitchMode(gdriver_ipc::SyncMode),
}

// ─── State transition logic ──────────────────────────────────────────────────

/// Valid transitions for the sync engine state machine.
///
/// Returns `true` when the transition is allowed and the state should change.
pub fn can_transition(current: SyncEngineState, next: SyncEngineState) -> bool {
    use SyncEngineState::*;
    match (current, next) {
        // Idle can go anywhere (initiated by events).
        (Idle, _) => true,
        // Scanning naturally flows into Syncing or back to Idle.
        (Scanning, Syncing | Idle) => true,
        // Syncing can go back to Idle (drained) or to Paused.
        (Syncing, Idle | Paused | Scanning) => true,
        // Paused can only go to Idle (via Resume).
        (Paused, Idle) => true,
        // Everything else is invalid.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── State → SyncStatus mapping ─────────────────────────────────────────

    #[test]
    fn idle_maps_to_up_to_date() {
        assert_eq!(
            SyncEngineState::Idle.to_sync_status(),
            gdriver_ipc::SyncStatus::UpToDate
        );
    }

    #[test]
    fn scanning_maps_to_syncing() {
        assert_eq!(
            SyncEngineState::Scanning.to_sync_status(),
            gdriver_ipc::SyncStatus::Syncing
        );
    }

    #[test]
    fn syncing_maps_to_syncing() {
        assert_eq!(
            SyncEngineState::Syncing.to_sync_status(),
            gdriver_ipc::SyncStatus::Syncing
        );
    }

    #[test]
    fn paused_maps_to_paused() {
        assert_eq!(
            SyncEngineState::Paused.to_sync_status(),
            gdriver_ipc::SyncStatus::Paused
        );
    }

    // ── State transitions ──────────────────────────────────────────────────

    #[test]
    fn idle_to_scanning_allowed() {
        assert!(can_transition(
            SyncEngineState::Idle,
            SyncEngineState::Scanning
        ));
    }

    #[test]
    fn idle_to_syncing_allowed() {
        assert!(can_transition(
            SyncEngineState::Idle,
            SyncEngineState::Syncing
        ));
    }

    #[test]
    fn idle_to_paused_allowed() {
        assert!(can_transition(
            SyncEngineState::Idle,
            SyncEngineState::Paused
        ));
    }

    #[test]
    fn idle_to_idle_allowed() {
        assert!(can_transition(SyncEngineState::Idle, SyncEngineState::Idle));
    }

    #[test]
    fn scanning_to_syncing_allowed() {
        assert!(can_transition(
            SyncEngineState::Scanning,
            SyncEngineState::Syncing
        ));
    }

    #[test]
    fn scanning_to_idle_allowed() {
        assert!(can_transition(
            SyncEngineState::Scanning,
            SyncEngineState::Idle
        ));
    }

    #[test]
    fn scanning_to_paused_not_allowed() {
        assert!(!can_transition(
            SyncEngineState::Scanning,
            SyncEngineState::Paused
        ));
    }

    #[test]
    fn syncing_to_idle_allowed() {
        assert!(can_transition(
            SyncEngineState::Syncing,
            SyncEngineState::Idle
        ));
    }

    #[test]
    fn syncing_to_paused_allowed() {
        assert!(can_transition(
            SyncEngineState::Syncing,
            SyncEngineState::Paused
        ));
    }

    #[test]
    fn syncing_to_scanning_allowed() {
        assert!(can_transition(
            SyncEngineState::Syncing,
            SyncEngineState::Scanning
        ));
    }

    #[test]
    fn paused_to_idle_allowed() {
        assert!(can_transition(
            SyncEngineState::Paused,
            SyncEngineState::Idle
        ));
    }

    #[test]
    fn paused_to_scanning_not_allowed() {
        assert!(!can_transition(
            SyncEngineState::Paused,
            SyncEngineState::Scanning
        ));
    }

    #[test]
    fn paused_to_syncing_not_allowed() {
        assert!(!can_transition(
            SyncEngineState::Paused,
            SyncEngineState::Syncing
        ));
    }

    #[test]
    fn paused_to_paused_not_allowed() {
        // No-op transitions should be skipped by the caller, not the state
        // machine, but we reject them here for safety.
        assert!(!can_transition(
            SyncEngineState::Paused,
            SyncEngineState::Paused
        ));
    }

    // ── Full happy-path sequence ───────────────────────────────────────────

    #[test]
    fn full_sync_cycle_sequence() {
        let sequence = vec![
            (SyncEngineState::Idle, SyncEngineState::Scanning, true),
            (SyncEngineState::Scanning, SyncEngineState::Syncing, true),
            (SyncEngineState::Syncing, SyncEngineState::Idle, true),
        ];
        for (from, to, expected) in sequence {
            assert_eq!(
                can_transition(from, to),
                expected,
                "transition from {from:?} to {to:?}"
            );
        }
    }

    #[test]
    fn pause_resume_cycle_sequence() {
        let sequence = vec![
            (SyncEngineState::Idle, SyncEngineState::Paused, true),
            (SyncEngineState::Paused, SyncEngineState::Idle, true),
        ];
        for (from, to, expected) in sequence {
            assert_eq!(
                can_transition(from, to),
                expected,
                "transition from {from:?} to {to:?}"
            );
        }
    }

    // ── SyncCommand equality ──────────────────────────────────────────────

    #[test]
    fn sync_command_pause_eq() {
        assert_eq!(SyncCommand::Pause, SyncCommand::Pause);
    }

    #[test]
    fn sync_command_resume_ne_pause() {
        assert_ne!(SyncCommand::Pause, SyncCommand::Resume);
    }

    #[test]
    fn switch_mode_commands_with_different_modes_are_not_equal() {
        let cmd1 = SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Stream);
        let cmd2 = SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Mirror);
        assert_ne!(cmd1, cmd2);
    }

    #[test]
    fn switch_mode_commands_with_same_mode_are_equal() {
        let cmd1 = SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Stream);
        let cmd2 = SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Stream);
        assert_eq!(cmd1, cmd2);
    }

    // ── Debug output ───────────────────────────────────────────────────────

    #[test]
    fn engine_state_debug() {
        // Ensure Debug is implemented (compile-time check).
        let states = [
            SyncEngineState::Idle,
            SyncEngineState::Scanning,
            SyncEngineState::Syncing,
            SyncEngineState::Paused,
        ];
        for s in &states {
            let debug_str = format!("{s:?}");
            assert!(!debug_str.is_empty());
        }
    }

    #[test]
    fn engine_state_clone_eq() {
        let s = SyncEngineState::Syncing;
        assert_eq!(s, s.clone());
        assert_eq!(s, s);
    }

    #[test]
    fn sync_command_debug() {
        let cmds = [
            SyncCommand::Pause,
            SyncCommand::Resume,
            SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Stream),
        ];
        for c in &cmds {
            let debug_str = format!("{c:?}");
            assert!(!debug_str.is_empty());
        }
    }
}
