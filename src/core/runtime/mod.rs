/*
 * Runtime owns side-effect execution for the headless core boundary. Concrete
 * inbound adapters should receive CoreInput/AppEvent/Snapshot contracts, while
 * runtime workers convert application service completion into CoreInput.
 */
pub mod driver;
mod input_mailbox;

pub use driver::{CoreEffectExecutor, CoreRuntime};
pub use input_mailbox::{CoreInputReceiver, CoreInputSender, core_input_channel};
