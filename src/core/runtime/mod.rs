/*
 * Runtime owns side-effect execution for the headless core boundary. Concrete
 * inbound adapters should receive CoreInput/AppEvent/Snapshot contracts, while
 * runtime workers convert application service completion into CoreInput.
 */
pub mod driver;

pub use driver::{CoreEffectExecutor, CoreRuntime};
