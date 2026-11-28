use super::assert_no_forbidden_references_in_paths;

#[test]
fn vault_restore_contract_stays_out_of_general_kernel_and_gateway_surfaces() {
    // 복원 평문과 키의 타입은 일반 Kernel 이벤트, 프로토콜, Gateway, Issue Core 일반 계약에 나올 수 없다 (#2415).
    assert_no_forbidden_references_in_paths(
        "general Kernel events, Gateway responses, and Issue Core contracts must not reach the Vault restore contract",
        &[
            "src/domain/kernel_event.rs",
            "src/domain/kernel_spec.rs",
            "crates/issue-core/src/issue_work.rs",
            "src/application/port/outbound/issue_core_port.rs",
            "src/application/port/outbound/kernel_event_log_port.rs",
            "src/application/port/outbound/kernel_gateway_port.rs",
            "src/application/port/inbound/kernel_command_port.rs",
            "src/composition/kernel_event_recorder.rs",
            "src/adapter/inbound/kernel_protocol",
            "src/adapter/inbound/gateway",
            "src/adapter/outbound/gateway_kernel_client.rs",
            "src/adapter/outbound/gateway_kernel_client",
        ],
        &[
            "crate::application::port::outbound::evidence_vault_port",
            "evidence_vault_port",
            "VaultPlaintext",
            "VaultKey",
            "VaultRestore",
            "VaultUserConfirmation",
            "evidence_vault_unlock",
            "VaultUnlock",
            "evidence_vault_restore",
            "VaultReconnect",
            "VaultRestorable",
            "evidence_vault_backup",
            "VaultBackup",
            "VaultRecoveryKey",
        ],
    );
}

#[test]
fn raw_issue_snapshot_stays_out_of_general_kernel_and_gateway_surfaces() {
    // 원격 게시본의 원시 값은 신뢰된 처리 경로만 쓴다. 일반 표면은 공유본만 가진다 (#2420).
    assert_no_forbidden_references_in_paths(
        "general Kernel events, Gateway responses, and Issue Core contracts must not reach the raw issue snapshot contract",
        &[
            "src/domain/kernel_event.rs",
            "src/domain/kernel_spec.rs",
            "crates/issue-core/src/issue_work.rs",
            "src/application/port/outbound/issue_core_port.rs",
            "src/application/port/outbound/kernel_event_log_port.rs",
            "src/application/port/outbound/kernel_gateway_port.rs",
            "src/application/port/inbound/kernel_command_port.rs",
            "src/composition/kernel_event_recorder.rs",
            "src/adapter/inbound/kernel_protocol",
            "src/adapter/inbound/gateway",
            "src/adapter/outbound/gateway_kernel_client.rs",
            "src/adapter/outbound/gateway_kernel_client",
        ],
        &["issue_snapshot_port", "IssueRawFields", "IssueSnapshot"],
    );
}
