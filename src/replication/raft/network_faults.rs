use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AppendDelayKind {
    Any,
    UniformMembership,
    LogIndex(u64),
}

#[derive(Default)]
pub(crate) struct FaultController {
    blocked_targets: RwLock<BTreeSet<u64>>,
    delayed_appends: Mutex<BTreeMap<(u64, AppendDelayKind), Arc<AppendDelay>>>,
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

    pub(crate) fn delay_append_entries_at(
        &self,
        target: u64,
        log_index: u64,
    ) -> Result<AppendDelayHandle, String> {
        self.delay_next(target, AppendDelayKind::LogIndex(log_index))
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
        delays.insert((target, kind), Arc::clone(&delay));
        Ok(handle)
    }

    pub(crate) fn append_delay_for(
        &self,
        target: u64,
        has_uniform_membership: bool,
        entry_log_indices: &[u64],
    ) -> Result<Option<Arc<AppendDelay>>, String> {
        let mut delays = self
            .delayed_appends
            .lock()
            .map_err(|error| format!("test network delay lock poisoned: {error}"))?;
        let mut released_delay = None;
        if has_uniform_membership {
            if let Some((delay, is_pending)) = Self::append_delay_for_key(
                &mut delays,
                (target, AppendDelayKind::UniformMembership),
            ) {
                if is_pending {
                    return Ok(Some(delay));
                }
                released_delay = Some(delay);
            }
        }
        for log_index in entry_log_indices {
            if let Some((delay, is_pending)) = Self::append_delay_for_key(
                &mut delays,
                (target, AppendDelayKind::LogIndex(*log_index)),
            ) {
                if is_pending {
                    return Ok(Some(delay));
                }
                released_delay.get_or_insert(delay);
            }
        }
        if let Some((delay, is_pending)) =
            Self::append_delay_for_key(&mut delays, (target, AppendDelayKind::Any))
        {
            if is_pending {
                return Ok(Some(delay));
            }
            released_delay.get_or_insert(delay);
        }
        Ok(released_delay)
    }

    fn append_delay_for_key(
        delays: &mut BTreeMap<(u64, AppendDelayKind), Arc<AppendDelay>>,
        key: (u64, AppendDelayKind),
    ) -> Option<(Arc<AppendDelay>, bool)> {
        let delay = Arc::clone(delays.get(&key)?);
        let is_pending = delay.is_pending();
        // Keep a pending gate so a retry cannot bypass the test fault.
        if !is_pending {
            delays.remove(&key);
        }
        Some((delay, is_pending))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppendDelayAction {
    Pending,
    Release,
    Cancel,
}

pub(crate) struct AppendDelay {
    entered: SyncSender<usize>,
    retried: SyncSender<()>,
    claimed: AtomicBool,
    state: Mutex<AppendDelayAction>,
    released: Condvar,
    completed: SyncSender<Result<(), String>>,
}

impl AppendDelay {
    fn new() -> (Arc<Self>, AppendDelayHandle) {
        let (entered, entered_rx) = mpsc::sync_channel(1);
        let (retried, retried_rx) = mpsc::sync_channel(1);
        let (completed, completed_rx) = mpsc::sync_channel(1);
        let delay = Arc::new(Self {
            entered,
            retried,
            claimed: AtomicBool::new(false),
            state: Mutex::new(AppendDelayAction::Pending),
            released: Condvar::new(),
            completed,
        });
        (
            Arc::clone(&delay),
            AppendDelayHandle {
                entered: entered_rx,
                retried: retried_rx,
                delay,
                action_sent: false,
                completed: completed_rx,
            },
        )
    }

    pub(crate) fn claim(&self) -> bool {
        let first_claim = !self.claimed.swap(true, Ordering::AcqRel);
        if !first_claim {
            let _ = self.retried.try_send(());
        }
        first_claim
    }

    pub(crate) fn pause(&self, entry_count: usize) -> bool {
        let _ = self.entered.try_send(entry_count);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while *state == AppendDelayAction::Pending {
            state = self
                .released
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        *state == AppendDelayAction::Release
    }

    fn is_pending(&self) -> bool {
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) == AppendDelayAction::Pending
    }

    fn set_action(&self, action: AppendDelayAction) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state == AppendDelayAction::Pending {
            *state = action;
            self.released.notify_all();
        }
    }

    pub(crate) fn complete(&self, result: Result<(), String>) {
        let _ = self.completed.try_send(result);
    }
}

pub(crate) struct AppendDelayHandle {
    entered: Receiver<usize>,
    retried: Receiver<()>,
    delay: Arc<AppendDelay>,
    action_sent: bool,
    completed: Receiver<Result<(), String>>,
}

impl AppendDelayHandle {
    pub(crate) fn wait_until_paused(&self, timeout: Duration) -> Result<usize, String> {
        self.entered
            .recv_timeout(timeout)
            .map_err(|error| format!("AppendEntries RPC was not paused: {error}"))
    }

    pub(crate) fn wait_until_retried(&self, timeout: Duration) -> Result<(), String> {
        self.retried
            .recv_timeout(timeout)
            .map(|_| ())
            .map_err(|error| format!("AppendEntries retry did not hit the pending delay: {error}"))
    }

    pub(crate) fn release(&mut self) {
        if !self.action_sent {
            self.delay.set_action(AppendDelayAction::Release);
            self.action_sent = true;
        }
    }

    pub(crate) fn cancel(&mut self) {
        if !self.action_sent {
            self.delay.set_action(AppendDelayAction::Cancel);
            self.action_sent = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_index_gate_does_not_shadow_a_later_pending_gate() {
        let controller = FaultController::default();
        let mut first = controller.delay_append_entries_at(7, 10).unwrap();
        let first_delay = controller
            .append_delay_for(7, false, &[10])
            .unwrap()
            .unwrap();
        assert!(first_delay.claim());
        first.release();

        let later = controller.delay_append_entries_at(7, 20).unwrap();
        let selected = controller
            .append_delay_for(7, false, &[10, 20])
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&selected, &later.delay));
        assert!(selected.claim());
    }

    #[test]
    fn released_gate_rejects_one_in_flight_retry_before_removal() {
        let controller = FaultController::default();
        let mut gate = controller.delay_append_entries_at(7, 10).unwrap();
        let first = controller
            .append_delay_for(7, false, &[10])
            .unwrap()
            .unwrap();
        assert!(first.claim());
        gate.release();

        let retry = controller
            .append_delay_for(7, false, &[10])
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &retry));
        assert!(!retry.claim());
        assert!(
            controller
                .append_delay_for(7, false, &[10])
                .unwrap()
                .is_none()
        );
    }
}
