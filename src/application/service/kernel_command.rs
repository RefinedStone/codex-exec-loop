use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::outbound::kernel_gateway_port::{
    KernelGatewayPort, KernelGatewayReply,
};
use crate::domain::kernel_spec::{CommandState, KernelCommandKind};

/// The answers to a runtime command that no Gateway kernel can take. The transport adapter builds
/// them, because the answer shape belongs to the protocol.
pub struct UnreachableAnswers {
    pub no_gateway: Value,
    pub held_elsewhere: Value,
}

/// The only routing policy shared by command clients. There are three ways (design §3.7): the
/// Gateway kernel, no Gateway, and a kernel that another process (a TUI) runs.
pub struct KernelCommandService {
    gateway: Arc<dyn KernelGatewayPort>,
    command_only: Arc<dyn KernelCommandPort>,
    unreachable: UnreachableAnswers,
}

impl KernelCommandService {
    pub fn new(
        gateway: Arc<dyn KernelGatewayPort>,
        command_only: Arc<dyn KernelCommandPort>,
        unreachable: UnreachableAnswers,
    ) -> Self {
        Self {
            gateway,
            command_only,
            unreachable,
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
        let reply = self.gateway.request("akra/kernel/command", params, false)?;
        // A store command runs under the repository lock without the Gateway kernel. The lock owner
        // tells that path whether a TUI holds the kernel. A runtime command needs the Gateway kernel.
        match (reply, kind.spec().state) {
            (KernelGatewayReply::Answer(answer), _) => Ok(answer),
            (_, CommandState::Store) => self.command_only.command(kind, input, idempotency_key),
            (KernelGatewayReply::NoGateway, CommandState::Runtime) => {
                Ok(self.unreachable.no_gateway.clone())
            }
            (KernelGatewayReply::KernelHeldElsewhere, CommandState::Runtime) => {
                Ok(self.unreachable.held_elsewhere.clone())
            }
        }
    }

    fn query(&self, input: Value, wait: bool) -> Result<Value> {
        match self
            .gateway
            .request("akra/kernel/query", json!({ "query": input }), wait)?
        {
            KernelGatewayReply::Answer(answer) => Ok(answer),
            KernelGatewayReply::NoGateway | KernelGatewayReply::KernelHeldElsewhere => {
                self.command_only.query(input, wait)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Copy, PartialEq)]
    enum Reach {
        Answers,
        NoGateway,
        HeldElsewhere,
    }

    struct Gateway {
        reach: Reach,
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
            Ok(match self.reach {
                Reach::Answers => KernelGatewayReply::Answer(json!({"remote":true})),
                Reach::NoGateway => KernelGatewayReply::NoGateway,
                Reach::HeldElsewhere => KernelGatewayReply::KernelHeldElsewhere,
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

    fn answers() -> UnreachableAnswers {
        UnreachableAnswers {
            no_gateway: json!({"rejection":"no gateway"}),
            held_elsewhere: json!({"rejection":"held elsewhere"}),
        }
    }

    #[test]
    fn the_gateway_kernel_is_preferred_and_a_runtime_command_never_runs_locally() {
        // Design §3.7: three ways. The answer to a runtime command names the true cause.
        for (reach, store_answer, runtime_answer) in [
            (
                Reach::Answers,
                json!({"remote":true}),
                json!({"remote":true}),
            ),
            (
                Reach::NoGateway,
                json!({"local":true}),
                json!({"rejection":"no gateway"}),
            ),
            (
                Reach::HeldElsewhere,
                json!({"local":true}),
                json!({"rejection":"held elsewhere"}),
            ),
        ] {
            let local = Arc::new(Local::default());
            let service = KernelCommandService::new(
                Arc::new(Gateway {
                    reach,
                    fails: false,
                }),
                local.clone(),
                answers(),
            );
            let store = service
                .command(KernelCommandKind::ResetPlanning, json!({}), None)
                .unwrap();
            assert_eq!(store, store_answer);
            let runtime = service
                .command(KernelCommandKind::SubmitPrompt, json!({}), None)
                .unwrap();
            assert_eq!(runtime, runtime_answer);
            assert_eq!(
                local.0.load(Ordering::SeqCst),
                usize::from(reach != Reach::Answers),
                "only the store command may run locally"
            );
        }
    }

    #[test]
    fn uncertain_gateway_failures_never_execute_locally() {
        let local = Arc::new(Local::default());
        let service = KernelCommandService::new(
            Arc::new(Gateway {
                reach: Reach::Answers,
                fails: true,
            }),
            local.clone(),
            answers(),
        );
        assert!(
            service
                .command(KernelCommandKind::ResetPlanning, json!({}), None)
                .is_err()
        );
        assert_eq!(local.0.load(Ordering::SeqCst), 0);
    }
}
