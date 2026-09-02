use super::{
    AdoptionManifest, BaseDomain, ColumnName, Connection, IndexName, PhysicalColumn, PhysicalIndex,
    PhysicalSchema, PhysicalTable, ProviderName, RevisionName, SchemaAttestation, SchemaName,
    SiteError, Symbol, TableName, map_error, provider_symbol, relation_id_text, valid_source,
};

/// Normalizes the main `SQLite` catalog, excluding capsule metadata.
///
/// # Errors
///
/// Returns a typed site refusal when catalog queries fail or observed names,
/// domains, columns, indexes, or revisions cannot be normalized.
pub fn introspect_connection(
    connection: &Connection,
    revision: RevisionName,
) -> Result<PhysicalSchema, SiteError> {
    let mut tables_stmt = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '__sim_%' ORDER BY name").map_err(|error| map_error(&error))?;
    let names = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| map_error(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_error(&error))?;
    let mut tables = Vec::new();
    for name in names {
        if !valid_source(&Symbol::new(name.as_str())) {
            return Err(SiteError::Conversion);
        }
        let mut columns_stmt = connection
            .prepare(&format!(
                "PRAGMA table_info(\"{}\")",
                name.replace('"', "\"\"")
            ))
            .map_err(|error| map_error(&error))?;
        let columns = columns_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| map_error(&error))?
            .map(|result| {
                let (ordinal, name, ty, notnull) = result.map_err(|error| map_error(&error))?;
                let (domain, storage) = affinity(&ty);
                Ok(PhysicalColumn {
                    name: ColumnName::new(Symbol::new(name)).map_err(|_| SiteError::Conversion)?,
                    domain: domain.id(),
                    storage,
                    nullable: notnull == 0,
                    ordinal: u32::try_from(ordinal).map_err(|_| SiteError::Conversion)?,
                })
            })
            .collect::<Result<Vec<_>, SiteError>>()?;
        let mut indexes_stmt = connection
            .prepare(&format!(
                "PRAGMA index_list(\"{}\")",
                name.replace('"', "\"\"")
            ))
            .map_err(|error| map_error(&error))?;
        let index_rows = indexes_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?;
        let mut indexes = Vec::new();
        for (index_name, unique) in index_rows {
            if index_name.starts_with("sqlite_") {
                continue;
            }
            let mut info = connection
                .prepare(&format!(
                    "PRAGMA index_info(\"{}\")",
                    index_name.replace('"', "\"\"")
                ))
                .map_err(|error| map_error(&error))?;
            let keys = info
                .query_map([], |row| row.get::<_, String>(2))
                .map_err(|error| map_error(&error))?
                .map(|v| {
                    ColumnName::new(Symbol::new(v.map_err(|error| map_error(&error))?))
                        .map_err(|_| SiteError::Conversion)
                })
                .collect::<Result<Vec<_>, _>>()?;
            indexes.push(PhysicalIndex {
                name: IndexName::new(Symbol::new(index_name)).map_err(|_| SiteError::Conversion)?,
                columns: keys,
                unique: unique != 0,
            });
        }
        tables.push(PhysicalTable {
            name: TableName::new(Symbol::new(name)).map_err(|_| SiteError::Conversion)?,
            columns,
            indexes,
        });
    }
    PhysicalSchema::normalize(
        ProviderName::new(provider_symbol()).map_err(|_| SiteError::Conversion)?,
        SchemaName::new(Symbol::new("main")).map_err(|_| SiteError::Conversion)?,
        revision,
        tables,
    )
    .map_err(|_| SiteError::Conversion)
}

/// Verifies existing capsule metadata or atomically adopts an exact old file.
///
/// Adoption cannot bless drift: the authored physical identity must equal a
/// fresh normalized catalog observation before metadata is written.
///
/// # Errors
///
/// Returns a typed site refusal when introspection fails, retained attestation
/// metadata disagrees, adoption proof is absent or invalid, or the metadata
/// transaction cannot commit atomically.
pub fn verify_or_adopt(
    connection: &mut Connection,
    logical_schema: sim_relation_core::RelationId,
    revision: sim_relation_core::RelationId,
    revision_name: RevisionName,
    adoption: Option<&AdoptionManifest>,
) -> Result<SchemaAttestation, SiteError> {
    let physical_schema = introspect_connection(connection, revision_name)?
        .id()
        .map_err(|_| SiteError::Provider)?;
    let existing = connection
        .query_row(
            "SELECT logical_schema, physical_schema, revision FROM __sim_relation_attestation WHERE singleton=1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .ok();
    if let Some((logical, physical, recorded_revision)) = existing {
        if logical != relation_id_text(&logical_schema)
            || physical != relation_id_text(&physical_schema)
            || recorded_revision != relation_id_text(&revision)
        {
            return Err(SiteError::Drift);
        }
    } else {
        adoption
            .ok_or(SiteError::Drift)?
            .verify(&physical_schema)
            .map_err(|_| SiteError::Drift)?;
        let transaction = connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        transaction.execute_batch("CREATE TABLE __sim_relation_attestation (singleton INTEGER PRIMARY KEY CHECK(singleton=1), logical_schema TEXT NOT NULL, physical_schema TEXT NOT NULL, revision TEXT NOT NULL)").map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "INSERT INTO __sim_relation_attestation VALUES (1, ?1, ?2, ?3)",
                (
                    relation_id_text(&logical_schema),
                    relation_id_text(&physical_schema),
                    relation_id_text(&revision),
                ),
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))?;
    }
    Ok(SchemaAttestation {
        logical_schema,
        physical_schema,
        revision,
    })
}
fn affinity(value: &str) -> (BaseDomain, sim_relation_core::StorageRepr) {
    let upper = value.to_ascii_uppercase();
    if upper.contains("INT") {
        (BaseDomain::I64, sim_relation_core::StorageRepr::I64)
    } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        (BaseDomain::Text, sim_relation_core::StorageRepr::Text)
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        (BaseDomain::F64, sim_relation_core::StorageRepr::F64)
    } else {
        (BaseDomain::Bytes, sim_relation_core::StorageRepr::Bytes)
    }
}
