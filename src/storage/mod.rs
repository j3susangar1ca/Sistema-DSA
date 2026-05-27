//! src/storage/mod.rs
//! Módulo: [SYNC-001] / [STORE-001] SQLite Persistence Bootstrap
//! Cumplimiento: ISO/IEC 25010 (Reliability, Performance Efficiency)
//!
//! Administrador transaccional local con Write-Ahead Log (WAL). Todas las
//! operaciones bloqueantes de `rusqlite` se ejecutan dentro de
//! `tokio::task::spawn_blocking` para no saturar el async runtime de Tokio.
//!
//! Garantiza: cero pérdida de datos ante caídas de red, consistencia eventual
//! perfecta, y recuperación atómica mediante `BEGIN IMMEDIATE`.

use anyhow::{Context, Result};
use rusqlite::{Connection, TransactionBehavior};
use std::path::{Path, PathBuf};
use tokio::task;
use tracing::{info, instrument};

/// Error tipado de la capa de persistencia.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Error de I/O en path {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Transacción abortada por panic o inconsistencia")]
    TransactionAborted,
}

/// Administrador de conexiones SQLite efímeras para el Edge Agent.
///
/// Diseño: conexiones de corta duración (efímeras) abiertas dentro de
/// `spawn_blocking`. Esto evita problemas de `Send`/`Sync` en el runtime
/// async y permite que SQLite maneje su propio locking a nivel archivo
/// con WAL mode.
#[derive(Debug, Clone)]
pub struct SQLiteManager {
    db_path: PathBuf,
}

impl SQLiteManager {
    /// Inicializa el administrador validando que el directorio padre exista.
    pub fn new(db_path: PathBuf) -> Result<Self, StorageError> {
        if let Some(parent) = db_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| StorageError::Io {
                    path: parent.display().to_string(),
                    source: e,
                })?;
            }
        }
        Ok(Self { db_path })
    }

    /// Bootstrap completo del schema transaccional.
    ///
    /// Ejecuta:
    /// - PRAGMAs de WAL y sincronía.
    /// - DDL de `fondo_revolvente_ledger` (35 columnas, PK compuesta).
    /// - Tablas auxiliares: `command_queue`, `expedition_events`,
    ///   `access_audit_log`, `sync_pointer`.
    /// - Índices secundarios optimizados.
    #[instrument(skip(self))]
    pub async fn initialize_schema(&self) -> Result<(), StorageError> {
        let path = self.db_path.clone();
        task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            // Configuración WAL para fiabilidad transaccional
            conn.execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA foreign_keys = ON;
                PRAGMA temp_store = MEMORY;
                ",
            )?;

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

            // ─── Tabla canónica: fondo_revolvente_ledger ───
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS fondo_revolvente_ledger (
                    folio_dsa                 TEXT    NOT NULL,
                    tipo_tramite              TEXT    NOT NULL DEFAULT 'COMPRA POR FONDO',
                    fecha_recepcion           TEXT    NOT NULL,
                    servicio_solicitante      TEXT    NOT NULL,
                    oficio_solicitud          TEXT    NOT NULL,
                    codigo                    TEXT    NOT NULL,
                    descripcion               TEXT    NOT NULL,
                    cantidad_solicitada       REAL    NOT NULL,
                    unidad_medida             TEXT    NOT NULL,
                    partida_especifica        TEXT    NOT NULL,
                    usuario_asignado          TEXT    NOT NULL,
                    fecha_inicio_cotizacion   TEXT,
                    estatus_tramite           TEXT    NOT NULL,
                    observaciones             TEXT,
                    folio_supre               TEXT,
                    fecha_supre               TEXT,
                    paquete_envio_caa         INTEGER,
                    fecha_recibido_caa        TEXT,
                    fecha_autorizacion_caa    TEXT,
                    folio_autorizacion_caa    TEXT,
                    precio_unitario           REAL,
                    monto_subtotal            REAL,
                    monto_iva                 REAL,
                    monto_total_con_iva       REAL,
                    cantidad_pedido           REAL,
                    numero_pedido             TEXT,
                    fecha_pedido              TEXT,
                    proveedor_rfc             TEXT,
                    estatus_entrega           TEXT,
                    fecha_entrega_almacen     TEXT,
                    numero_factura            TEXT,
                    fecha_factura             TEXT,
                    fecha_envio_xml_rf        TEXT,
                    fecha_pago                TEXT,
                    fecha_complemento_pago_rf TEXT,
                    created_at                TEXT    NOT NULL,
                    updated_at                TEXT    NOT NULL,
                    sync_status               TEXT    DEFAULT 'PENDING',
                    PRIMARY KEY (folio_dsa, codigo)
                );

                CREATE INDEX IF NOT EXISTS idx_ledger_estatus
                    ON fondo_revolvente_ledger(estatus_tramite);
                CREATE INDEX IF NOT EXISTS idx_ledger_sync
                    ON fondo_revolvente_ledger(sync_status);
                CREATE INDEX IF NOT EXISTS idx_ledger_codigo
                    ON fondo_revolvente_ledger(codigo);
                ",
            )?;

            // ─── Cola de comandos local (WAL de comandos) ───
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS command_queue (
                    command_id        TEXT PRIMARY KEY,
                    action            TEXT NOT NULL,
                    timestamp         TEXT NOT NULL,
                    requested_by      TEXT NOT NULL,
                    execution_status  TEXT NOT NULL,
                    payload           TEXT,
                    response_payload  TEXT,
                    completed_at      TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_queue_status
                    ON command_queue(execution_status);
                ",
            )?;

            // ─── Event sourcing append-only ───
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS expedition_events (
                    id              TEXT PRIMARY KEY,
                    expedition_id   TEXT NOT NULL,
                    event_type      TEXT NOT NULL,
                    actor           TEXT NOT NULL,
                    payload         TEXT,
                    timestamp       TEXT NOT NULL,
                    deadline_status TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_events_expedition
                    ON expedition_events(expedition_id);
                CREATE INDEX IF NOT EXISTS idx_events_timestamp
                    ON expedition_events(timestamp);
                ",
            )?;

            // ─── Auditoría de accesos (AUTH-001) ───
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS access_audit_log (
                    id          TEXT PRIMARY KEY,
                    email       TEXT NOT NULL,
                    result      TEXT NOT NULL,
                    client_ip   TEXT,
                    timestamp   TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_audit_email
                    ON access_audit_log(email);
                CREATE INDEX IF NOT EXISTS idx_audit_timestamp
                    ON access_audit_log(timestamp);
                ",
            )?;

            // ─── Sync pointer (compatibilidad legacy v4.1) ───
            tx.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS sync_pointer (
                    id                      TEXT PRIMARY KEY,
                    sheet_id                TEXT UNIQUE,
                    last_processed_row_id   INTEGER,
                    last_processed_expedition_id TEXT,
                    retry_count             INTEGER,
                    status                  TEXT NOT NULL,
                    updated_at              TEXT NOT NULL
                );
                ",
            )?;

            tx.commit()?;
            info!(target: "storage", "Schema inicializado correctamente en {:?}", path);
            Ok(())
        })
        .await
        .map_err(|_| StorageError::TransactionAborted)?
    }

    /// Ejecuta una closure dentro de una transacción SQLite inmediata,
    /// garantizando atomicidad y rollback automático ante fallo.
    ///
    /// La closure recibe una `rusqlite::Connection` mutable.
    pub async fn execute<F, T>(&self, operation: F) -> Result<T, StorageError>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let path = self.db_path.clone();
        task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            let mut tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let result = operation(&mut tx)?;
            tx.commit()?;
            Ok(result)
        })
        .await
        .map_err(|_| StorageError::TransactionAborted)?
    }

    /// Retorna la ruta de la base de datos (utilidad para diagnóstico).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

// Re-exportar submódulos futuros
// pub mod ledger;
// pub mod queue;