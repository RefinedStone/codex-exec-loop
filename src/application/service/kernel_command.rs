use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::outbound::kernel_gateway_port::{
    KernelGatewayPort, KernelGatewayReply,
};
use crate::domain::kernel_spec::{CommandState, KernelCommandKind};

/// The only routing policy shared by command clients.
pub struct KernelCommandService {
    gateway: Arc<dyn KernelGatewayPort>,
    command_only: Arc<dyn KernelCommandPort>,
    not_running: Value,
}

impl KernelCommandService {
    pub fn new(
        gateway: Arc<dyn KernelGatewayPort>,
        command_only: Arc<dyn KernelCommandPort>,
        not_running: Value,
    ) -> Self {
        Self {
            gateway,
            command_only,
            not_running,
        }
    }
}

impl KernelCommandPort for KernelCommandService {
    fn command(
        &self,
        kind: KernelCommandKind,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        let params = json!({ "command": input, "idempotency_key": idempotency_key });
        match self.gateway.request("akra/kernel/command", params, false)? {
            KernelGatewayReply::Answer(answer) => Ok(answer),
            KernelGatewayReply::Unavailable => match kind.spec().state {
                CommandState::Store => self.command_only.command(kind, input, idempotency_key),
                CommandState::Runtime => Ok(self.not_running.clone()),
            },
        }
    }

    fn query(&self, input: Value, wait: bool) -> Result<Value> {
        match self
            .gateway
            .request("akra/kernel/query", json!({ "query": input }), wait)?
        {
            KernelGatewayReply::Answer(answer) => Ok(answer),
            KernelGatewayReply::Unavailable => self.command_only.query(input, wait),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Gateway {
        available: bool,
        fails: bool,
    }
    impl KernelGatewayPort for Gateway {
        fn request(
            &self,
            _method: &str,
            _params: Value,
            _wait: bool,
        ) -> Result<KernelGatewayReply> {
            if self.fails {
                bail!("connection lost after send");
            }
            Ok(if self.available {
                KernelGatewayReply::Answer(json!({"remote":true}))
            } else {
                KernelGatewayReply::Unavailable
            })
        }
    }
    #[derive(Default)]
    struct Local(AtomicUsize);
    impl KernelCommandPort for Local {
        fn command(
            &self,
            _kind: KernelCommandKind,
            _input: Value,
            _key: Option<String>,
        ) -> Result<Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"local":true}))
        }
        fn query(&self, _input: Value, _wait: bool) -> Result<Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"local":true}))
        }
    }

    #[test]
    fn gateway_is_preferred_and_absent_runtime_is_rejected_without_local_execution() {
        for available in [false, true] {
            let local = Arc::new(Local::default());
            let service = KernelCommandService::new(
                Arc::new(Gateway {
                    available,
                    fails: false,
                }),
                local.clone(),
                json!({"rejection":{"reason":"kernel_not_running"}}),
            );
            let store = service
                .command(KernelCommandKind::ResetPlanning, json!({}), None)
                .unwrap();
            assert_eq!(
                store,
                if available {
                    json!({"remote":true})
                } else {
                    json!({"local":true})
                }
            );
            let runtime = service
                .command(KernelCommandKind::SubmitPrompt, json!({}), None)
                .unwrap();
            assert_eq!(
                runtime,
                if available {
                    json!({"remote":true})
                } else {
                    json!({"rejection":{"reason":"kernel_not_running"}})
                }
            );
            assert_eq!(local.0.load(Ordering::SeqCst), usize::from(!available));
        }
    }

    #[test]
    fn uncertain_gateway_failures_never_execute_locally() {
        let local = Arc::new(Local::default());
        let service = KernelCommandService::new(
            Arc::new(Gateway {
                available: true,
                fails: true,
            }),
            local.clone(),
            Value::Null,
        );
        assert!(
            service
                .command(KernelCommandKind::ResetPlanning, json!({}), None)
                .is_err()
        );
        assert_eq!(local.0.load(Ordering::SeqCst), 0);
    }
}
