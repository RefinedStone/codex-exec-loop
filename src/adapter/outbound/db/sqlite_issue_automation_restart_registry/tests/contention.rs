use super::*;

#[test]
fn registration_retries_after_another_writer_releases_its_lock() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let original = fixture.entry(1);
    let added = fixture.entry(2);
    adapter.register(&original).unwrap();
    let mut opened =
        crate::private_sqlite::open_existing(&fixture.runtime, FILES, Synchronous::Extra)
            .unwrap()
            .unwrap();
    let (connection, _guard) = opened.parts();
    let mut holder = Some(
        connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap(),
    );
    let mut attempts = 0;
    retry_busy(CONTENTION_RETRY_WINDOW, |_| {
        attempts += 1;
        let result = adapter.register_once(&fixture.runtime, &added, Duration::ZERO);
        if attempts == 1 {
            let error = result.as_ref().unwrap_err();
            assert!(is_busy(error), "{error:#}");
            assert_eq!(
                error.to_string(),
                "failed to begin restart registry transaction"
            );
            holder.take().unwrap().rollback().unwrap();
        }
        result
    })
    .unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(fixture.adapter().list().unwrap(), vec![original, added]);
}

#[test]
fn commit_contention_rolls_back_before_retrying_registration() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let original = fixture.entry(1);
    let added = fixture.entry(2);
    adapter.register(&original).unwrap();
    let mut opened =
        crate::private_sqlite::open_existing(&fixture.runtime, FILES, Synchronous::Extra)
            .unwrap()
            .unwrap();
    let (connection, _guard) = opened.parts();
    let mut reader = Some(
        connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .unwrap(),
    );
    // BEGIN alone takes no read lock. Materialize a snapshot that blocks COMMIT.
    assert_eq!(
        read_entries(reader.as_ref().unwrap()).unwrap(),
        vec![original.clone()]
    );
    let mut attempts = 0;
    retry_busy(CONTENTION_RETRY_WINDOW, |_| {
        attempts += 1;
        let result = adapter.register_once(&fixture.runtime, &added, Duration::ZERO);
        if attempts == 1 {
            let error = result.as_ref().unwrap_err();
            assert!(is_busy(error), "{error:#}");
            assert_eq!(
                error.to_string(),
                "failed to commit restart registry transaction"
            );
            // Keep the reader alive: a new writer must acquire the released reservation
            // and see that the failed INSERT was rolled back before the retry starts.
            let mut verifier = crate::private_sqlite::open_existing_with_busy_timeout(
                &fixture.runtime,
                FILES,
                Synchronous::Extra,
                Duration::ZERO,
            )
            .unwrap()
            .unwrap();
            let (connection, _guard) = verifier.parts();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            assert_eq!(read_entries(&transaction).unwrap(), vec![original.clone()]);
            transaction.rollback().unwrap();
            reader.take().unwrap().rollback().unwrap();
        }
        result
    })
    .unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(fixture.adapter().list().unwrap(), vec![original, added]);
}

#[test]
fn exhausted_contention_window_returns_busy_without_partial_registration() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let original = fixture.entry(1);
    adapter.register(&original).unwrap();
    let mut opened =
        crate::private_sqlite::open_existing(&fixture.runtime, FILES, Synchronous::Extra)
            .unwrap()
            .unwrap();
    let (connection, _guard) = opened.parts();
    let holder = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let mut attempts = 0;
    let error = retry_busy(Duration::ZERO, |busy_timeout| {
        attempts += 1;
        adapter.register_once(&fixture.runtime, &fixture.entry(2), busy_timeout)
    })
    .unwrap_err();
    assert!(is_busy(&error), "{error:#}");
    assert_eq!(attempts, 1);
    holder.rollback().unwrap();
    assert_eq!(fixture.adapter().list().unwrap(), vec![original]);
}

#[test]
fn unsupported_schema_is_not_retried_as_contention() {
    let fixture = Fixture::new();
    fixture.adapter().register(&fixture.entry(1)).unwrap();
    fixture.edit(|connection| {
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    });
    let mut attempts = 0;
    let error = retry_busy(CONTENTION_RETRY_WINDOW, |busy_timeout| {
        attempts += 1;
        fixture.adapter().list_once(&fixture.runtime, busy_timeout)
    })
    .unwrap_err();
    assert!(!is_busy(&error));
    assert!(format!("{error:#}").contains("identity or schema version is unsupported"));
    assert_eq!(attempts, 1);
}
