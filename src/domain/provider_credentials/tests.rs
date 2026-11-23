use super::*;

fn connection(provider: Provider, token_bytes: usize) -> StoredConnection {
    StoredConnection {
        summary: ConnectionSummary {
            reference: ConnectionRef {
                profile_id: "a".repeat(64),
                provider,
                connection_id: "b".repeat(32),
                revision: 1,
            },
            metadata: ConnectionMetadata {
                configuration: match provider {
                    Provider::Linear => ConnectionConfiguration::default(),
                    Provider::Jira => ConnectionConfiguration {
                        site_url: Some("https://example.atlassian.net".into()),
                        deployment: Some(JiraDeployment::Cloud),
                        email: Some("person@example.test".into()),
                    },
                },
                account_id: "account".into(),
                account_name: "Person".into(),
            },
        },
        operation_id: "test-operation".into(),
        part_count: token_bytes.div_ceil(SECRET_CHUNK_BYTES) as u16,
    }
}

#[test]
fn linear_preserves_the_existing_utf16_limit_instead_of_limiting_utf8_bytes() {
    let cjk = "界".repeat(MAX_LINEAR_UTF16_UNITS);
    assert_eq!(cjk.len(), MAX_LINEAR_SECRET_BYTES);
    let token = SecretToken::from_bytes(Provider::Linear, cjk.as_bytes().to_vec())
        .unwrap_or_else(|_| panic!("valid legacy bound"));
    assert_eq!(token.as_str(), cjk);
    assert!(matches!(
        SecretToken::from_bytes(
            Provider::Linear,
            "界".repeat(MAX_LINEAR_UTF16_UNITS + 1).into_bytes()
        ),
        Err(ProviderCredentialError::TooLarge)
    ));
    assert!(SecretToken::from_bytes(Provider::Linear, "😀".repeat(2048).into_bytes()).is_ok());
    assert!(matches!(
        SecretToken::from_bytes(Provider::Linear, "😀".repeat(2049).into_bytes()),
        Err(ProviderCredentialError::TooLarge)
    ));
    assert!(matches!(
        SecretToken::from_bytes(
            Provider::Linear,
            format!(" {}", "a".repeat(4096)).into_bytes()
        ),
        Err(ProviderCredentialError::TooLarge)
    ));
}

#[test]
fn token_validation_rejects_invalid_utf8_and_embedded_ascii_whitespace_and_bounds_jira() {
    for value in [
        vec![0xff],
        b"".to_vec(),
        b"  ".to_vec(),
        b"aa bb".to_vec(),
        b"aa\nbb".to_vec(),
        b"aa\0bb".to_vec(),
    ] {
        assert!(SecretToken::from_bytes(Provider::Linear, value).is_err());
    }
    let value = SecretToken::from_bytes(Provider::Jira, b"  synthetic-token  ".to_vec())
        .unwrap_or_else(|_| panic!("valid trimmed token"));
    assert_eq!(value.expose(), b"synthetic-token");
    assert!(SecretToken::from_bytes(Provider::Jira, vec![b'x'; MAX_JIRA_SECRET_BYTES]).is_ok());
    assert!(matches!(
        SecretToken::from_bytes(Provider::Jira, vec![b'x'; MAX_JIRA_SECRET_BYTES + 1]),
        Err(ProviderCredentialError::TooLarge)
    ));
}

#[test]
fn public_intents_are_flat_and_reject_secret_or_unknown_fields() {
    let request = ConnectRequest {
        operation_id: "operation".into(),
        profile_id: "a".repeat(64),
        provider: Provider::Linear,
        expected_revision: 0,
        configuration: ConnectionConfiguration::default(),
    };
    let mut value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["expectedRevision"], 0);
    assert_eq!(value["profileId"], request.profile_id);
    value["token"] = "must-never-be-an-intent".into();
    assert!(serde_json::from_value::<ConnectRequest>(value).is_err());
    assert!(
        serde_json::from_str::<ConnectionConfiguration>(
            r#"{"siteUrl":null,"password":"forbidden"}"#
        )
        .is_err()
    );
}

#[test]
fn normalized_configuration_is_provider_scoped() {
    let mut jira = connection(Provider::Jira, 1).summary.metadata.configuration;
    jira.site_url = Some("http://example.test".into());
    assert!(jira.normalized(Provider::Jira).is_err());
    jira.site_url = Some("https://user:password@example.test".into());
    assert!(jira.normalized(Provider::Jira).is_err());
    assert!(jira.normalized(Provider::Linear).is_err());
    assert!(
        ConnectionConfiguration::default()
            .normalized(Provider::Jira)
            .is_err()
    );
}

#[test]
fn multipart_capsules_round_trip_the_maximum_and_stay_below_windows_blob_limit() {
    for (provider, length) in [
        (Provider::Linear, MAX_LINEAR_SECRET_BYTES),
        (Provider::Jira, MAX_JIRA_SECRET_BYTES),
    ] {
        let raw = if provider == Provider::Linear {
            "界".repeat(4096).into_bytes()
        } else {
            vec![b'z'; length]
        };
        let token = SecretToken::from_bytes(provider, raw)
            .unwrap_or_else(|_| panic!("valid synthetic token"));
        let connection = connection(provider, token.expose().len());
        let capsules = encode_secret_capsules(&connection, &token).unwrap();
        assert_eq!(capsules.len(), usize::from(connection.part_count));
        assert!(capsules.iter().all(|part| part.len() <= 2560));
        let decoded = decode_secret_capsules(&connection, &capsules)
            .unwrap_or_else(|_| panic!("valid capsule"));
        assert_eq!(decoded.expose(), token.expose());
    }
}

#[test]
fn capsules_bind_profile_provider_generation_connection_operation_metadata_and_part_order() {
    let token = SecretToken::from_bytes(Provider::Linear, vec![b'x'; 4096])
        .unwrap_or_else(|_| panic!("fixture"));
    let original = connection(Provider::Linear, 4096);
    let capsules = encode_secret_capsules(&original, &token).unwrap();
    let mut variants = Vec::new();
    let mut changed = original.clone();
    changed.summary.reference.profile_id = "c".repeat(64);
    variants.push(changed);
    let mut changed = original.clone();
    changed.summary.reference.connection_id = "d".repeat(32);
    variants.push(changed);
    let mut changed = original.clone();
    changed.summary.reference.revision += 1;
    variants.push(changed);
    let mut changed = original.clone();
    changed.operation_id = "other".into();
    variants.push(changed);
    let mut changed = original.clone();
    changed.summary.metadata.account_name = "Other".into();
    variants.push(changed);
    for changed in variants {
        assert!(matches!(
            decode_secret_capsules(&changed, &capsules),
            Err(ProviderCredentialError::Corrupt)
        ));
    }
    let mut reordered = encode_secret_capsules(&original, &token).unwrap();
    reordered.swap(0, 1);
    assert!(decode_secret_capsules(&original, &reordered).is_err());
    let mut corrupted = encode_secret_capsules(&original, &token).unwrap();
    corrupted[1][CAPSULE_HEADER] ^= 1;
    assert!(decode_secret_capsules(&original, &corrupted).is_err());
    assert!(decode_secret_capsules(&original, &capsules[..1]).is_err());
}

#[test]
fn ledger_rejects_invalid_identity_and_hidden_fields() {
    let mut ledger = CredentialLedger::empty("a".repeat(64), Provider::Linear);
    assert!(ledger.validate().is_ok());
    ledger.profile_id = "other".into();
    assert_eq!(ledger.validate(), Err(ProviderCredentialError::Corrupt));
    let mut value =
        serde_json::to_value(CredentialLedger::empty("a".repeat(64), Provider::Linear)).unwrap();
    value["tokenHash"] = "forbidden".into();
    assert!(serde_json::from_value::<CredentialLedger>(value).is_err());
}
