use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AppendDelayKind {
    Any,
    UniformMembership,
}

#[derive(Default)]
pub(crate) struct FaultController {
    blocked_targets: RwLock<BTreeSet<u64>>,
    delayed_appends: Mutex<BTreeMap<(u64, AppendDelayKind), AppendDelay>>,
}

impl FaultController {
    pub(crate) fn set_peer_blocked(&self, target: u64, blocked: bool) -> Result<(), String> {
        let mut targets = self
            .blocked_targets
            .write()
            .map_err(|error| format!("test network fault lock poisoned: {error}"))?;
        if blocked {
            targets.insert(target);
        } else {
            targets.remove(&target);
        }
        Ok(())
    }

    pub(crate) fn check_network(&self, target: u64, operation: &str) -> Result<(), String> {
        let blocked = self
            .blocked_targets
            .read()
            .map_err(|error| format!("test network fault lock poisoned: {error}"))?
            .contains(&target);
        if blocked {
            Err(format!("test network dropped {operation} RPC"))
        } else {
            Ok(())
        }
    }

    pub(crate) fn delay_next_append_entries(
        &self,
        target: u64,
    ) -> Result<AppendDelayHandle, String> {
        self.delay_next(target, AppendDelayKind::Any)
    }

    pub(crate) fn delay_next_uniform_membership_append(
        &self,
        target: u64,
    ) -> Result<AppendDelayHandle, String> {
        self.delay_next(target, AppendDelayKind::UniformMembership)
    }

    fn delay_next(&self, target: u64, kind: AppendDelayKind) -> Result<AppendDelayHandle, String> {
        let (delay, handle) = AppendDelay::new();
        let mut delays = self
            .delayed_appends
            .lock()
            .map_err(|error| format!("test network delay lock poisoned: {error}"))?;
        if delays.contains_key(&(target, kind)) {
            return Err(format!(
                "an AppendEntries delay is already armed for peer {target}"
            ));
        }
        delays.insert((target, kind), delay);
        Ok(handle)
    }

    pub(crate) fn take_append_delay(
        &self,
        target: u64,
        has_uniform_membership: bool,
    ) -> Result<Option<AppendDelay>, String> {
        let mut delays = self
            .delayed_appends
            .lock()
            .map_err(|error| format!("test network delay lock poisoned: {error}"))?;
        if has_uniform_membership {
            if let Some(delay) = delays.remove(&(target, AppendDelayKind::UniformMembership)) {
                return Ok(Some(delay));
            }
        }
        Ok(delays.remove(&(target, AppendDelayKind::Any)))
    }
}

enum AppendDelayAction {
    Release,
    Cancel,
}

pub(crate) struct AppendDelay {
    entered: SyncSender<()>,
    release: Receiver<AppendDelayAction>,
    completed: SyncSender<Result<(), String>>,
}

impl AppendDelay {
    fn new() -> (Self, AppendDelayHandle) {
        let (entered, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release) = mpsc::sync_channel(1);
        let (completed, completed_rx) = mpsc::sync_channel(1);
        (
            Self {
                entered,
                release,
                completed,
            },
            AppendDelayHandle {
                entered: entered_rx,
                release: Some(release_tx),
                completed: completed_rx,
            },
        )
    }

    pub(crate) fn pause(&self) -> bool {
        let _ = self.entered.send(());
        matches!(self.release.recv(), Ok(AppendDelayAction::Release))
    }

    pub(crate) fn complete(self, result: Result<(), String>) {
        let _ = self.completed.send(result);
    }
}

pub(crate) struct AppendDelayHandle {
    entered: Receiver<()>,
    release: Option<SyncSender<AppendDelayAction>>,
    completed: Receiver<Result<(), String>>,
}

impl AppendDelayHandle {
    pub(crate) fn wait_until_paused(&self, timeout: Duration) -> Result<(), String> {
        self.entered
            .recv_timeout(timeout)
            .map_err(|error| format!("AppendEntries RPC was not paused: {error}"))
    }

    pub(crate) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(AppendDelayAction::Release);
        }
    }

    pub(crate) fn cancel(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(AppendDelayAction::Cancel);
        }
    }

    pub(crate) fn wait_for_completion(&self, timeout: Duration) -> Result<(), String> {
        self.completed
            .recv_timeout(timeout)
            .map_err(|error| format!("delayed AppendEntries RPC did not complete: {error}"))?
    }
}

impl Drop for AppendDelayHandle {
    fn drop(&mut self) {
        self.release();
    }
}
