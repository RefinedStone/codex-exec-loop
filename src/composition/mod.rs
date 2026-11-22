/*
 * composition은 process-level production wiring root다.
 *
 * application layer는 concrete adapter를 알면 안 되고, inbound adapter도 production
 * outbound graph를 직접 조립하지 않는다. 이 top-level 모듈만 실제 outbound adapter를
 * application port/service 계약에 연결한다.
 */
pub(crate) mod clipboard_image_probe;
pub(crate) mod core_effect_runner;
pub(crate) mod core_effect_worker;
pub(crate) mod core_turn_submission;
pub(crate) mod desktop_restore;
pub(crate) mod issue_collection;
pub(crate) mod issue_confirmation;
pub(crate) mod issue_observer;
pub(crate) mod issue_work_execution;
pub(crate) mod kernel_command_execution;
pub(crate) mod kernel_event_recorder;
pub(crate) mod kernel_host;
pub(crate) mod kernel_routed_planning_admin;
pub(crate) mod native_client_runtime;
pub(crate) mod native_terminal_event_ingress;
mod pr_review;
pub(crate) mod production;
pub(crate) mod repository_extensions;
pub(crate) mod repository_kernel;
pub(crate) mod repository_observer;
