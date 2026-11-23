//! 한 저장소의 모든 실행 graph가 공유하는 바인딩별 실행 진입 경계다.
//! 외부 요청 동안에는 소유한 guard만 유지하며 mutex를 잡지 않는다.
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseEvent, IssueBindingPauseOutcome, IssueBindingPauseState,
};
use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
struct BindingAdmission {
    generation: u64,
    closed: bool,
    in_flight: usize,
    pending_command_id: Option<String>,
}

#[derive(Debug, Default)]
struct AdmissionState {
    closed: bool,
    bindings: HashMap<IssueAutomationBindingId, BindingAdmission>,
}

pub struct IssueBindingAdmission {
    state: Mutex<AdmissionState>,
    waker: Mutex<Option<Waker>>,
}

impl Default for IssueBindingAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for IssueBindingAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssueBindingAdmission")
            .finish_non_exhaustive()
    }
}

impl IssueBindingAdmission {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AdmissionState::default()),
            waker: Mutex::new(None),
        }
    }

    pub fn set_waker(&self, waker: Waker) {
        if let Ok(mut slot) = self.waker.lock() {
            *slot = Some(waker);
        }
    }

    fn wake(&self) {
        let callback = self.waker.lock().ok().and_then(|slot| slot.clone());
        if let Some(callback) = callback {
            callback();
        }
    }

    /// 초기 복구와 새 바인딩 등록에만 사용한다. 이후의 오래된 조회는 문을 다시 열지 못한다.
    pub fn register_if_absent(&self, observed: &IssueBindingPauseState) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        if state.closed {
            bail!("issue binding runtime is closed");
        }
        match state.bindings.get_mut(&observed.binding_id) {
            None => {
                state.bindings.insert(
                    observed.binding_id.clone(),
                    BindingAdmission {
                        generation: observed.revision,
                        closed: !observed.allows_automation(),
                        in_flight: 0,
                        pending_command_id: observed
                            .pending
                            .as_ref()
                            .map(|intent| intent.command_id.clone()),
                    },
                );
            }
            Some(current) => {
                // A later durable closed state may tighten admission. Only completion below
                // can reopen an existing entry; a delayed read cannot undo an accepted Pause.
                if observed.revision >= current.generation && !observed.allows_automation() {
                    current.generation = observed.revision;
                    current.closed = true;
                    current.pending_command_id = observed
                        .pending
                        .as_ref()
                        .map(|intent| intent.command_id.clone());
                } else if observed.revision > current.generation {
                    bail!("issue binding admission requires current runtime recovery");
                }
            }
        }
        Ok(())
    }

    pub fn capture(&self, binding_id: &IssueAutomationBindingId) -> Result<u64> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        let binding = open_binding(&state, binding_id)?;
        Ok(binding.generation)
    }

    pub fn acquire(
        self: &Arc<Self>,
        binding_id: &IssueAutomationBindingId,
        generation: u64,
    ) -> Result<IssueBindingActionGuard> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        let current = open_binding(&state, binding_id)?;
        if current.generation != generation {
            bail!("issue binding automation generation is stale");
        }
        let current = state.bindings.get_mut(binding_id).expect("validated entry");
        current.in_flight = current
            .in_flight
            .checked_add(1)
            .ok_or_else(|| anyhow!("issue binding in-flight count exhausted"))?;
        Ok(IssueBindingActionGuard {
            admission: self.clone(),
            binding_id: binding_id.clone(),
            generation,
        })
    }

    /// 저장과 메모리 차단 사이에 새 guard가 들어오지 못하게 한다.
    pub(crate) fn begin_transition<T>(
        &self,
        binding_id: &IssueAutomationBindingId,
        persist: impl FnOnce() -> Result<(T, IssueBindingPauseState)>,
    ) -> Result<T> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        if state.closed {
            bail!("issue binding runtime is closed");
        }
        let (result, stored) = match persist() {
            Ok(value) => value,
            Err(error) => {
                // A persistence error cannot prove that a commit did not happen. Close an
                // existing entry conservatively; an unchanged Resume must not reopen it.
                if let Some(entry) = state.bindings.get_mut(binding_id) {
                    entry.closed = true;
                    entry.pending_command_id = None;
                }
                drop(state);
                self.wake();
                return Err(error);
            }
        };
        if stored.binding_id != *binding_id {
            bail!("issue binding pause persistence returned another binding");
        }
        let entry = state
            .bindings
            .entry(binding_id.clone())
            .or_insert(BindingAdmission {
                generation: stored.revision,
                closed: !stored.allows_automation(),
                in_flight: 0,
                pending_command_id: stored
                    .pending
                    .as_ref()
                    .map(|intent| intent.command_id.clone()),
            });
        if stored.revision < entry.generation {
            bail!("issue binding pause persistence returned an older generation");
        }
        let unchanged = entry.generation == stored.revision;
        entry.generation = stored.revision;
        entry.closed = !stored.allows_automation() || (unchanged && entry.closed);
        entry.pending_command_id = stored
            .pending
            .as_ref()
            .map(|intent| intent.command_id.clone());
        drop(state);
        self.wake();
        Ok(result)
    }

    /// 이미 시작한 효과의 ACK/Unknown이 확정될 때까지 완료 영수증을 만들지 않는다.
    pub(crate) fn finish_if_drained<T>(
        &self,
        binding_id: &IssueAutomationBindingId,
        generation: u64,
        persist: impl FnOnce() -> Result<(T, IssueBindingPauseState)>,
    ) -> Result<Option<T>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        if state.closed {
            bail!("issue binding runtime is closed");
        }
        let entry = state
            .bindings
            .get(binding_id)
            .ok_or_else(|| anyhow!("issue binding admission was not restored"))?;
        if entry.generation != generation || !entry.closed {
            bail!("issue binding pause completion generation is stale");
        }
        if entry.in_flight != 0 {
            return Ok(None);
        }
        let (result, stored) = persist()?;
        if stored.binding_id != *binding_id || stored.revision != generation {
            bail!("issue binding completion persistence changed its generation");
        }
        let entry = state.bindings.get_mut(binding_id).expect("validated entry");
        entry.closed = !stored.allows_automation();
        entry.pending_command_id = stored
            .pending
            .as_ref()
            .map(|intent| intent.command_id.clone());
        drop(state);
        self.wake();
        Ok(Some(result))
    }

    /// 완료 transaction의 응답을 잃어도 같은 pending의 최초 영수증으로만 복구한다.
    pub(crate) fn reconcile_completed_event(
        &self,
        stored: &IssueBindingPauseState,
        event: &IssueBindingPauseEvent,
    ) -> Result<()> {
        if stored.revision != event.receipt.revision
            || stored.binding_id != event.receipt.binding_id
            || stored.paused != event.receipt.paused
            || stored.pending.is_some()
        {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        if state.closed {
            bail!("issue binding runtime is closed");
        }
        let Some(entry) = state.bindings.get_mut(&stored.binding_id) else {
            bail!("issue binding admission was not restored");
        };
        if entry.generation != stored.revision
            || entry.pending_command_id.as_deref() != Some(event.receipt.command_id.as_str())
        {
            return Ok(());
        }
        if entry.in_flight != 0 {
            bail!("issue binding completion has outstanding effects");
        }
        if stored.allows_automation() && event.receipt.outcome != IssueBindingPauseOutcome::Resumed
        {
            bail!("only the accepted resume completion can reopen issue binding admission");
        }
        entry.closed = !stored.allows_automation();
        entry.pending_command_id = None;
        Ok(())
    }

    pub fn has_inflight(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.bindings.values().any(|entry| entry.in_flight != 0))
            .unwrap_or(true)
    }

    pub fn close_all(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.wake();
    }
}

fn open_binding<'a>(
    state: &'a AdmissionState,
    binding_id: &IssueAutomationBindingId,
) -> Result<&'a BindingAdmission> {
    if state.closed {
        bail!("issue binding runtime is closed");
    }
    let entry = state
        .bindings
        .get(binding_id)
        .ok_or_else(|| anyhow!("issue binding admission was not restored"))?;
    if entry.closed {
        bail!("issue binding automation is paused or pending");
    }
    Ok(entry)
}

#[derive(Debug)]
pub struct IssueBindingActionGuard {
    admission: Arc<IssueBindingAdmission>,
    binding_id: IssueAutomationBindingId,
    generation: u64,
}

impl IssueBindingActionGuard {
    /// 외부 효과 직전에 사용한다. DB·파일·네트워크를 읽지 않는다.
    pub fn validate(&self) -> Result<()> {
        let state = self
            .admission
            .state
            .lock()
            .map_err(|_| anyhow!("issue binding admission is unavailable"))?;
        if open_binding(&state, &self.binding_id)?.generation != self.generation {
            bail!("issue binding automation generation is stale");
        }
        Ok(())
    }
}

impl Drop for IssueBindingActionGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.admission.state.lock()
            && let Some(entry) = state.bindings.get_mut(&self.binding_id)
        {
            // A pause advances the generation while old requests drain. Their guards still
            // belong to this binding's aggregate count, not to the new authorization epoch.
            entry.in_flight = entry.in_flight.saturating_sub(1);
        }
        self.admission.wake();
    }
}

#[cfg(test)]
mod tests;
