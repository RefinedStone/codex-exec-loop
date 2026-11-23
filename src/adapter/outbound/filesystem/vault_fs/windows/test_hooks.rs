use std::cell::RefCell;

use anyhow::{Result, bail};

thread_local! {
    static BEFORE_REPLACE: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static AFTER_REPLACE_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    static AFTER_RECORD_READ: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static REJECT_VOLUME: RefCell<bool> = const { RefCell::new(false) };
}

pub(crate) fn install_before_atomic_replace_hook(hook: impl FnOnce() + 'static) {
    BEFORE_REPLACE.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "the previous vault hook was not consumed"
        );
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

pub(crate) fn install_after_atomic_replace_error_hook(message: impl Into<String>) {
    AFTER_REPLACE_ERROR.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "the previous vault hook was not consumed"
        );
        *slot.borrow_mut() = Some(message.into());
    });
}

pub(super) fn before_replace() {
    let hook = BEFORE_REPLACE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

pub(super) fn after_replace() -> Result<()> {
    if let Some(message) = AFTER_REPLACE_ERROR.with(|slot| slot.borrow_mut().take()) {
        bail!(message);
    }
    Ok(())
}

pub(super) fn reject_next_volume_admission() {
    REJECT_VOLUME.with(|slot| *slot.borrow_mut() = true);
}

pub(super) fn check_volume_admission() -> Result<()> {
    if REJECT_VOLUME.with(|slot| slot.replace(false)) {
        bail!("the vault requires writable local NTFS with persistent ACLs");
    }
    Ok(())
}

pub(super) fn install_after_record_read_hook(hook: impl FnOnce() + 'static) {
    AFTER_RECORD_READ.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "the previous vault read hook was not consumed"
        );
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

pub(super) fn after_record_read() {
    let hook = AFTER_RECORD_READ.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
