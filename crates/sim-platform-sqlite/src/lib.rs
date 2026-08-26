//! Attesting `SQLite` realization behind the provider-neutral relation site.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use rusqlite::{
    Connection, OpenFlags, ToSql,
    types::{FromSql, Value, ValueRef},
};
use sim_codec_sql::{
    PreparedSql, SqlBinding, SqliteDialect, prepare_migration, prepare_mutation, prepare_query,
};
use sim_kernel::{Datum, LibManifest, Symbol};
use sim_relation_core::{
    BaseDomain, Cell, ColumnName, DomainCatalog, DomainId, IndexName, ProviderName, RevisionName,
    Row, SchemaName, StorageValue, TableName,
};
use sim_relation_migrate::{AdoptionManifest, CheckedProgram, SchemaAttestation};
use sim_relation_plan::{CheckedMutation, CheckedQuery};
use sim_relation_schema::{PhysicalColumn, PhysicalIndex, PhysicalSchema, PhysicalTable};
use sim_relation_site::{
    Bindings, Driver, DriverManifest, Limits, ProviderStats, RelationPlacement, RelationSite,
    RelationSiteLib, RowSink, Session, SiteError, StorageAccess, StorageLocator, Transaction,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Capsule-owned map from stable preopened names to private host paths.
#[derive(Clone, Default)]
pub struct PreopenedStores(Arc<BTreeMap<Symbol, PathBuf>>);
impl PreopenedStores {
    /// Builds the capsule authority map. Paths never enter the relation locator.
    pub fn new(values: impl IntoIterator<Item = (Symbol, PathBuf)>) -> Self {
        Self(Arc::new(values.into_iter().collect()))
    }
    fn resolve(&self, name: &Symbol) -> Option<&Path> {
        self.0.get(name).map(PathBuf::as_path)
    }
}

/// The sole `SQLite` driver, configured with admitted domains and preopened storage.
pub struct SqliteDriver {
    domains: Arc<DomainCatalog>,
    stores: PreopenedStores,
    busy_ms: u32,
}
impl SqliteDriver {
    /// Constructs the capsule driver.
    #[must_use]
    pub fn new(domains: DomainCatalog, stores: PreopenedStores) -> Self {
        Self {
            domains: Arc::new(domains),
            stores,
            busy_ms: 2_000,
        }
    }
    /// Constructs the canonical loadable site library.
    ///
    /// # Errors
    ///
    /// Returns a registration refusal when the canonical driver manifest is
    /// invalid.
    pub fn library(self, locator: &StorageLocator) -> Result<RelationSiteLib, SiteError> {
        let manifest = DriverManifest::sqlite(site_symbol(), provider_symbol())?;
        let datum = locator_datum(locator);
        Ok(RelationSiteLib::new(RelationSite::new(
            RelationPlacement::new(manifest.site, datum),
            Arc::new(self),
        )))
    }
}

/// Canonical exported site symbol.
#[must_use]
pub fn site_symbol() -> Symbol {
    Symbol::qualified("relation/site", "sqlite")
}
/// Canonical provider identity.
#[must_use]
pub fn provider_symbol() -> Symbol {
    Symbol::qualified("relation/provider", "sqlite")
}
/// Verifies that a manifest declares exactly the `SQLite` kernel site export.
///
/// # Errors
///
/// Returns [`SiteError::Registration`] unless exactly one canonical site export
/// is declared.
pub fn verify_manifest(manifest: &LibManifest) -> Result<(), SiteError> {
    let count = manifest.exports.iter().filter(|export| matches!(export, sim_kernel::Export::Site { symbol, .. } if symbol == &site_symbol())).count();
    if count == 1 {
        Ok(())
    } else {
        Err(SiteError::Registration)
    }
}

fn locator_datum(value: &StorageLocator) -> Datum {
    match value {
        StorageLocator::Memory => Datum::Node {
            tag: Symbol::qualified("relation", "memory"),
            fields: vec![],
        },
        StorageLocator::Preopened { reference, access } => Datum::Node {
            tag: Symbol::qualified("relation", "preopened"),
            fields: vec![
                (Symbol::new("ref"), Datum::Symbol(reference.clone())),
                (
                    Symbol::new("access"),
                    Datum::Symbol(Symbol::new(match access {
                        StorageAccess::ReadOnly => "read-only",
                        StorageAccess::ReadWrite => "read-write",
                    })),
                ),
            ],
        },
    }
}

impl Driver for SqliteDriver {
    fn connect(&self, locator: &Datum, limits: &Limits) -> Result<Box<dyn Session>, SiteError> {
        let locator = StorageLocator::from_datum(locator)?;
        let connection = match locator {
            StorageLocator::Memory => Connection::open_in_memory(),
            StorageLocator::Preopened { reference, access } => {
                let path = self.stores.resolve(&reference).ok_or(SiteError::Locator)?;
                let flags = match access {
                    StorageAccess::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY,
                    StorageAccess::ReadWrite => {
                        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
                    }
                };
                Connection::open_with_flags(path, flags)
            }
        }
        .map_err(|error| map_error(&error))?;
        configure(&connection, self.busy_ms, limits)?;
        Ok(Box::new(SqliteSession {
            connection,
            domains: self.domains.clone(),
            stores: self.stores.clone(),
            cache: HashMap::new(),
            generation: 0,
            savepoint: 0,
        }))
    }
}

fn configure(connection: &Connection, busy_ms: u32, limits: &Limits) -> Result<(), SiteError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| map_error(&error))?;
    connection
        .busy_timeout(std::time::Duration::from_millis(u64::from(busy_ms)))
        .map_err(|error| map_error(&error))?;
    let started = Instant::now();
    let deadline = limits.deadline;
    connection.progress_handler(
        100,
        Some(move || deadline.is_some_and(|limit| started.elapsed() >= limit)),
    );
    Ok(())
}

struct SqliteSession {
    connection: Connection,
    domains: Arc<DomainCatalog>,
    stores: PreopenedStores,
    cache: HashMap<String, String>,
    generation: u64,
    savepoint: u64,
}
impl SqliteSession {
    fn execute_rows(
        &mut self,
        prepared: &PreparedSql,
        bindings: &Bindings,
        limits: &Limits,
        sink: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        let key = format!("{:?}:{}", prepared.cache_key(), self.generation);
        self.cache
            .entry(key)
            .or_insert_with(|| prepared.text().to_owned());
        let values = bind_values(prepared, bindings, &self.domains)?;
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v as &dyn ToSql).collect();
        let mut statement = self
            .connection
            .prepare_cached(prepared.text())
            .map_err(|error| map_error(&error))?;
        if prepared.cache_key().output_row_type.fields().is_empty() {
            let affected = statement
                .execute(refs.as_slice())
                .map_err(|error| map_error(&error))? as u64;
            return Ok(ProviderStats {
                work: affected.max(1),
                affected,
            });
        }
        let mut rows = statement
            .query(refs.as_slice())
            .map_err(|error| map_error(&error))?;
        let mut work = 0u64;
        while let Some(row) = rows.next().map_err(|error| map_error(&error))? {
            work = work
                .checked_add(1)
                .ok_or(SiteError::Limit(sim_relation_site::LimitKind::Work))?;
            if work > limits.work {
                return Err(SiteError::Limit(sim_relation_site::LimitKind::Work));
            }
            let cells = prepared
                .cache_key()
                .output_row_type
                .fields()
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    decode_cell(
                        row.get_ref(index).map_err(|error| map_error(&error))?,
                        &field.domain,
                        &self.domains,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let admitted = Row::new(prepared.cache_key().output_row_type.clone(), cells)
                .map_err(|_| SiteError::Conversion)?;
            sink.push(admitted)?;
        }
        Ok(ProviderStats { work, affected: 0 })
    }
    fn migrate_inner(
        &mut self,
        program: &CheckedProgram,
        limits: &Limits,
    ) -> Result<ProviderStats, SiteError> {
        let catalog = program
            .program()
            .base_schema
            .id()
            .map_err(|_| SiteError::Provider)?;
        let statements = prepare_migration(program, &catalog, &SqliteDialect)
            .map_err(|_| SiteError::Provider)?;
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| map_error(&error))?;
        let result = (|| {
            let empty = sim_relation_core::RowType::new([]).map_err(|_| SiteError::Provider)?;
            let bindings = Bindings::new(&empty, []).map_err(|_| SiteError::Provider)?;
            let mut sink = NullSink;
            let mut stats = ProviderStats::default();
            for statement in statements.statements() {
                let got = self.execute_rows(statement, &bindings, limits, &mut sink)?;
                stats.work += got.work;
                stats.affected += got.affected;
            }
            self.write_attestation(program)?;
            Ok(stats)
        })();
        match result {
            Ok(stats) => {
                self.connection
                    .execute_batch("COMMIT")
                    .map_err(|error| map_error(&error))?;
                self.invalidate();
                Ok(stats)
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
    fn write_attestation(&self, program: &CheckedProgram) -> Result<(), SiteError> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS __sim_relation_attestation (singleton INTEGER PRIMARY KEY CHECK(singleton=1), logical_schema TEXT NOT NULL, physical_schema TEXT NOT NULL, revision TEXT NOT NULL)").map_err(|error| map_error(&error))?;
        let physical = self.introspect(
            RevisionName::new(Symbol::new("current")).map_err(|_| SiteError::Conversion)?,
        )?;
        let logical = relation_id_text(&program.program().target_schema);
        let revision_id = program
            .program()
            .revisions
            .last()
            .map_or(&program.program().base_revision, |r| r.id());
        let revision = relation_id_text(revision_id);
        let physical_id = physical.id().map_err(|_| SiteError::Provider)?;
        self.connection
            .execute(
                "INSERT OR REPLACE INTO __sim_relation_attestation VALUES (1, ?1, ?2, ?3)",
                (&logical, relation_id_text(&physical_id), revision),
            )
            .map_err(|error| map_error(&error))?;
        Ok(())
    }
    fn invalidate(&mut self) {
        self.connection.flush_prepared_statement_cache();
        self.cache.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    fn introspect(&self, revision: RevisionName) -> Result<PhysicalSchema, SiteError> {
        introspect_connection(&self.connection, revision)
    }
}

impl Session for SqliteSession {
    fn query(
        &mut self,
        plan: &CheckedQuery,
        bindings: &Bindings,
        limits: &Limits,
        sink: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        let prepared = prepare_query(plan, &SqliteDialect).map_err(|_| SiteError::Provider)?;
        self.execute_rows(&prepared, bindings, limits, sink)
    }
    fn mutate(
        &mut self,
        plan: &CheckedMutation,
        bindings: &Bindings,
        limits: &Limits,
        sink: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        let prepared = prepare_mutation(plan, &SqliteDialect).map_err(|_| SiteError::Provider)?;
        self.execute_rows(&prepared, bindings, limits, sink)
    }
    fn migrate(
        &mut self,
        program: &CheckedProgram,
        limits: &Limits,
    ) -> Result<ProviderStats, SiteError> {
        self.migrate_inner(program, limits)
    }
    fn schema(
        &mut self,
        program: &CheckedProgram,
        limits: &Limits,
    ) -> Result<ProviderStats, SiteError> {
        self.migrate_inner(program, limits)
    }
    fn transaction(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| map_error(&error))?;
        match body(self) {
            Ok(()) => self
                .connection
                .execute_batch("COMMIT")
                .map_err(|error| map_error(&error)),
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
    fn attach(&mut self, locator: &Datum, _: &Limits) -> Result<ProviderStats, SiteError> {
        let Datum::Node { tag, fields } = locator else {
            return Err(SiteError::Locator);
        };
        if tag != &Symbol::qualified("relation", "attach") || fields.len() != 3 {
            return Err(SiteError::Locator);
        }
        let get = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| name == &Symbol::new(key))
                .map(|(_, value)| value)
        };
        let (
            Some(Datum::Symbol(name)),
            Some(Datum::Symbol(reference)),
            Some(Datum::Symbol(access)),
        ) = (get("name"), get("ref"), get("access"))
        else {
            return Err(SiteError::Locator);
        };
        if !valid_source(name) {
            return Err(SiteError::Locator);
        }
        let path = self.stores.resolve(reference).ok_or(SiteError::Locator)?;
        if access.namespace.is_some() {
            return Err(SiteError::Locator);
        }
        let uri = match access.name.as_ref() {
            "read-only" => format!("file:{}?mode=ro", path.display()),
            "read-write" => path.display().to_string(),
            _ => return Err(SiteError::Locator),
        };
        self.connection
            .execute("ATTACH DATABASE ?1 AS ?2", (&uri, name.name.as_ref()))
            .map_err(|error| map_error(&error))?;
        self.invalidate();
        Ok(ProviderStats {
            work: 1,
            affected: 0,
        })
    }
}
impl Transaction for SqliteSession {
    fn savepoint(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError> {
        self.savepoint += 1;
        let name = format!("sim_savepoint_{}", self.savepoint);
        self.connection
            .execute_batch(&format!("SAVEPOINT {name}"))
            .map_err(|error| map_error(&error))?;
        match body(self) {
            Ok(()) => self
                .connection
                .execute_batch(&format!("RELEASE {name}"))
                .map_err(|error| map_error(&error)),
            Err(error) => {
                let _ = self
                    .connection
                    .execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
                Err(error)
            }
        }
    }
}

struct NullSink;
impl RowSink for NullSink {
    fn push(&mut self, _: Row) -> Result<(), SiteError> {
        Ok(())
    }
}

fn bind_values(
    prepared: &PreparedSql,
    supplied: &Bindings,
    domains: &DomainCatalog,
) -> Result<Vec<Value>, SiteError> {
    prepared
        .bindings()
        .iter()
        .map(|binding| match binding {
            SqlBinding::Literal(cell) => encode_cell(cell, domains),
            SqlBinding::Parameter(name) => supplied
                .row()
                .row_type()
                .fields()
                .iter()
                .position(|field| field.name.symbol() == name.symbol())
                .map(|index| encode_cell(&supplied.row().cells()[index], domains))
                .ok_or(SiteError::Conversion)?,
        })
        .collect()
}
fn base(domain: &DomainId, domains: &DomainCatalog) -> Result<BaseDomain, SiteError> {
    let storage = domains.get(domain).ok_or(SiteError::Conversion)?.storage();
    Ok(match storage {
        sim_relation_core::StorageRepr::Bool => BaseDomain::Bool,
        sim_relation_core::StorageRepr::I64 => BaseDomain::I64,
        sim_relation_core::StorageRepr::F64 => BaseDomain::F64,
        sim_relation_core::StorageRepr::Text => BaseDomain::Text,
        sim_relation_core::StorageRepr::Bytes => BaseDomain::Bytes,
    })
}
fn encode_cell(cell: &Cell, domains: &DomainCatalog) -> Result<Value, SiteError> {
    let Some(value) = cell.value() else {
        return Ok(Value::Null);
    };
    Ok(
        match base(cell.domain(), domains)?
            .from_datum(value)
            .map_err(|_| SiteError::Conversion)?
        {
            StorageValue::Bool(v) => Value::Integer(i64::from(v)),
            StorageValue::I64(v) => Value::Integer(v),
            StorageValue::F64(v) => Value::Real(v),
            StorageValue::Text(v) => Value::Text(v),
            StorageValue::Bytes(v) => Value::Blob(v),
        },
    )
}
fn decode_cell(
    value: ValueRef<'_>,
    domain: &DomainId,
    domains: &DomainCatalog,
) -> Result<Cell, SiteError> {
    if value == ValueRef::Null {
        return Ok(Cell::null(domain.clone()));
    }
    let base = base(domain, domains)?;
    let storage = match base {
        BaseDomain::Bool => {
            StorageValue::Bool(i64::column_result(value).map_err(|_| SiteError::Conversion)? != 0)
        }
        BaseDomain::I64 => {
            StorageValue::I64(i64::column_result(value).map_err(|_| SiteError::Conversion)?)
        }
        BaseDomain::F64 => {
            StorageValue::F64(f64::column_result(value).map_err(|_| SiteError::Conversion)?)
        }
        BaseDomain::Text => {
            StorageValue::Text(String::column_result(value).map_err(|_| SiteError::Conversion)?)
        }
        BaseDomain::Bytes => {
            StorageValue::Bytes(Vec::<u8>::column_result(value).map_err(|_| SiteError::Conversion)?)
        }
    };
    Ok(Cell::new(
        domain.clone(),
        Some(base.to_datum(storage).map_err(|_| SiteError::Conversion)?),
    ))
}
fn valid_source(value: &Symbol) -> bool {
    if value.namespace.is_some() {
        return false;
    }
    let text = value.name.as_ref();
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn relation_id_text(value: &sim_relation_core::RelationId) -> String {
    let content = value.content_id();
    let mut digest = String::with_capacity(content.bytes.len() * 2);
    for byte in content.bytes {
        digest.push(char::from(HEX[usize::from(byte >> 4)]));
        digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    format!("{}:{digest}", content.algorithm)
}
fn map_error(error: &rusqlite::Error) -> SiteError {
    use rusqlite::{
        Error::SqliteFailure,
        ffi::ErrorCode::{
            ConstraintViolation, DatabaseBusy, DatabaseCorrupt, DatabaseLocked, NotADatabase,
            OperationInterrupted, ReadOnly,
        },
    };
    match error {
        SqliteFailure(inner, _) => match inner.code {
            ConstraintViolation => SiteError::Constraint,
            DatabaseBusy | DatabaseLocked => SiteError::Locked,
            ReadOnly => SiteError::ReadOnly,
            OperationInterrupted => SiteError::Interrupted,
            DatabaseCorrupt | NotADatabase => SiteError::Corruption,
            _ => SiteError::Provider,
        },
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..) => SiteError::Conversion,
        _ => SiteError::Provider,
    }
}

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

#[cfg(test)]
mod tests {
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
        let read_only =
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
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
            .execute_batch(
                "CREATE TABLE source(value INTEGER UNIQUE); INSERT INTO source VALUES(4);",
            )
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

        connection.execute_batch("CREATE TABLE unique_value(value INTEGER UNIQUE); INSERT INTO unique_value VALUES(1);").unwrap();
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
}
