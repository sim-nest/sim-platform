use super::*;
use sim_kernel::Lib;

fn revision() -> RevisionName {
    RevisionName::new(Symbol::new("fixture-v1")).unwrap()
}
fn fixture_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sim-sqlite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn manifest_and_locators_are_closed() {
    let domains = DomainCatalog::new([
        BaseDomain::Bool.spec(),
        BaseDomain::I64.spec(),
        BaseDomain::F64.spec(),
        BaseDomain::Text.spec(),
        BaseDomain::Bytes.spec(),
    ])
    .unwrap();
    let lib = SqliteDriver::new(domains, PreopenedStores::default())
        .library(&StorageLocator::Memory)
        .unwrap();
    verify_manifest(&lib.manifest()).unwrap();
    assert_eq!(lib.manifest().exports.len(), 1);
}

#[test]
fn real_file_introspection_is_normalized_and_reopens() {
    let dir = fixture_dir("reopen");
    let path = dir.join("relation.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE item(flag INTEGER NOT NULL, count INTEGER, ratio REAL, label TEXT, payload BLOB);
                 CREATE UNIQUE INDEX item_label ON item(label);
                 INSERT INTO item VALUES(1, 7, 1.5, 'kept', X'0102');",
            ).unwrap();
        let physical = introspect_connection(&connection, revision()).unwrap();
        assert_eq!(physical.tables().len(), 1);
        let columns = &physical.tables()[0].columns;
        assert_eq!(columns.len(), 5);
        assert_eq!(columns[0].storage, sim_relation_core::StorageRepr::I64);
        assert_eq!(columns[2].storage, sim_relation_core::StorageRepr::F64);
        assert_eq!(columns[3].storage, sim_relation_core::StorageRepr::Text);
        assert_eq!(columns[4].storage, sim_relation_core::StorageRepr::Bytes);
        assert_eq!(physical.tables()[0].indexes.len(), 1);
        let physical_id = physical.id().unwrap();
        let manifest = AdoptionManifest {
            logical_schema: physical_id.clone(),
            physical_schema: physical_id.clone(),
        };
        drop(physical);
        drop(connection);
        let mut connection = Connection::open(&path).unwrap();
        let receipt = verify_or_adopt(
            &mut connection,
            physical_id.clone(),
            physical_id.clone(),
            revision(),
            Some(&manifest),
        )
        .unwrap();
        assert_eq!(receipt.physical_schema, physical_id);
        verify_or_adopt(
            &mut connection,
            receipt.logical_schema.clone(),
            receipt.revision.clone(),
            revision(),
            None,
        )
        .unwrap();
        connection
            .execute_batch("CREATE TABLE drifted(value TEXT)")
            .unwrap();
        assert!(matches!(
            verify_or_adopt(
                &mut connection,
                receipt.logical_schema,
                receipt.revision,
                revision(),
                None,
            ),
            Err(SiteError::Drift)
        ));
    }
    let read_only = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        read_only
            .query_row("SELECT label FROM item", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "kept"
    );
    assert!(matches!(
        read_only
            .execute("DELETE FROM item", [])
            .map_err(|error| map_error(&error)),
        Err(SiteError::ReadOnly)
    ));
}

#[test]
fn attach_lifecycle_and_stable_errors_are_real_sqlite_behaviour() {
    let dir = fixture_dir("attach");
    let attached = dir.join("attached.sqlite");
    Connection::open(&attached)
        .unwrap()
        .execute_batch("CREATE TABLE source(value INTEGER UNIQUE); INSERT INTO source VALUES(4);")
        .unwrap();
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute(
            "ATTACH DATABASE ?1 AS source_a",
            [attached.to_string_lossy().as_ref()],
        )
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM source_a.source", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
    connection
        .execute_batch("DETACH DATABASE source_a")
        .unwrap();
    assert!(connection.prepare("SELECT * FROM source_a.source").is_err());

    connection
        .execute_batch(
            "CREATE TABLE unique_value(value INTEGER UNIQUE); INSERT INTO unique_value VALUES(1);",
        )
        .unwrap();
    assert!(matches!(
        connection
            .execute("INSERT INTO unique_value VALUES(1)", [])
            .map_err(|error| map_error(&error)),
        Err(SiteError::Constraint)
    ));
    assert!(matches!(
        Connection::open(&dir).map_err(|error| map_error(&error)),
        Err(SiteError::Provider)
    ));
}

#[test]
fn generated_conflict_transaction_union_and_interruption_are_bounded() {
    let dir = fixture_dir("semantics");
    let left = dir.join("left.sqlite");
    let right = dir.join("right.sqlite");
    for (path, value) in [(&left, "left"), (&right, "right")] {
        let connection = Connection::open(path).unwrap();
        connection
                .execute_batch("CREATE TABLE item(id INTEGER PRIMARY KEY, flag INTEGER NOT NULL, ratio REAL, label TEXT UNIQUE, payload BLOB)")
                .unwrap();
        connection
            .execute(
                "INSERT INTO item(flag, ratio, label, payload) VALUES(1, 2.5, ?1, X'CAFE')",
                [value],
            )
            .unwrap();
        let generated = connection.last_insert_rowid();
        assert!(generated > 0);
        assert_eq!(
                connection
                    .execute(
                        "INSERT INTO item(flag, ratio, label, payload) VALUES(1, NULL, ?1, X'') ON CONFLICT(label) DO NOTHING",
                        [value],
                    )
                    .unwrap(),
                0
            );
    }

    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute(
            "ATTACH DATABASE ?1 AS left_source",
            [left.to_string_lossy().as_ref()],
        )
        .unwrap();
    connection
        .execute(
            "ATTACH DATABASE ?1 AS right_source",
            [right.to_string_lossy().as_ref()],
        )
        .unwrap();
    let grouped = connection.query_row(
            "SELECT COUNT(*), SUM(flag) FROM (SELECT flag FROM left_source.item UNION ALL SELECT flag FROM right_source.item)",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        ).unwrap();
    assert_eq!(grouped, (2, 2));

    connection
        .execute_batch("CREATE TABLE local(value INTEGER)")
        .unwrap();
    let mut transaction = connection.transaction().unwrap();
    transaction
        .execute("INSERT INTO local VALUES(1)", [])
        .unwrap();
    {
        let mut savepoint = transaction.savepoint().unwrap();
        savepoint
            .execute("INSERT INTO local VALUES(2)", [])
            .unwrap();
        savepoint.rollback().unwrap();
    }
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row("SELECT SUM(value) FROM local", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );

    connection.progress_handler(1, Some(|| true));
    let interrupted = connection.query_row(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT SUM(x) FROM n",
            [],
            |row| row.get::<_, i64>(0),
        );
    assert!(matches!(
        interrupted.map_err(|error| map_error(&error)),
        Err(SiteError::Interrupted)
    ));
}
