/*
 * Runtime drives the headless core boundary behind a composition-owned facade.
 * Inbound adapters receive CoreInput/AppEvent/Snapshot contracts without raw
 * driver, executor, or mailbox construction capability.
 */
mod driver;
mod input_mailbox;

pub(crate) use driver::{CoreEffectExecutor, CoreRuntime};
pub(crate) use input_mailbox::{CoreInputSender, core_input_channel};
