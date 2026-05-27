//! src/main.rs
//! SISTEMA-DSA: AGENTE DE EJECUCIÓN EDGE v2.0 Mission-Critical
//! Cumplimiento: ISO/IEC 25010 (Reliability, Maintainability, Portability)
//!
//! Bootstrap de componentes, inicialización de schema SQLite WAL,
//! lanzamiento de tareas concurrentes Tokio (polling + sync), y
//! graceful shutdown ante señales del sistema operativo.

mod config;
mod models;
mod protocol;
mod security;
mod storage;
mod sync;
mod utils;

use crate::config::EdgeConfig;
use crate::protocol::ProtocolEngine;
use crate::security::audit::AuditLogger;
use crate::storage::SQLiteManager;
use crate::sync::SyncBridge;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ─── 1. Logging estructurado JSON ───
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("sistema_dsa=info".parse()?)
            .add_directive("protocol=debug".parse()?)
            .add_directive("sync=info".parse()?)
            .add_directive("excel=warn".parse()?))
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!(target: "main", "====================================================");
    info!(target: "main", "SISTEMA-DSA: AGENTE DE EJECUCIÓN EDGE (WINDOWS 11)");
    info!(target: "main", "Protocolo Cloud-Edge v2.0 Mission-Critical");
    info!(target: "main", "Cumplimiento de Calidad ISO/IEC 25010");
    info!(target: "main", "====================================================");

    // ─── 2. Carga de configuración inmutable ───
    let config = EdgeConfig::load()
        .map_err(|e| {
            error!(target: "main", "Fallo crítico en configuración: {}", e);
            e
        })?;

    info!(target: "main", "Configuración cargada:");
    info!(target: "main", "  Control Plane: {}", config.control_plane_url);
    info!(target: "main", "  SQLite: {}", config.sqlite_path.display());
    info!(target: "main", "  Excel SMB: {}", config.excel_smb_path.display());
    info!(target: "main", "  BigQuery: {}.{}.{}", config.bq_project_id, config.bq_dataset, config.bq_table_ledger);

    // ─── 3. Inicialización de persistencia SQLite ───
    let sqlite = SQLiteManager::new(config.sqlite_path.clone());
    sqlite.initialize_schema().await
        .map_err(|e| {
            error!(target: "main", "Fallo al inicializar schema SQLite: {}", e);
            e
        })?;

    info!(target: "main", "Schema SQLite WAL inicializado correctamente.");

    // ─── 4. Inicialización de auditoría no bloqueante ───
    let audit = AuditLogger::new(sqlite.clone());

    // ─── 5. Construcción de motores ───
    let protocol_engine = ProtocolEngine::new(
        config.clone(),
        sqlite.clone(),
        audit.clone(),
    );

    let sync_bridge = SyncBridge::new(
        config.clone(),
        sqlite.clone(),
    );

    // ─── 6. Lanzamiento de tareas concurrentes ───
    info!(target: "main", "Desplegando tareas concurrentes...");

    let protocol_handle = tokio::spawn(async move {
        info!(target: "main", "[TAREA] ProtocolEngine iniciado (polling inverso)");
        protocol_engine.start_polling_loop().await
    });

    let sync_handle = tokio::spawn(async move {
        info!(target: "main", "[TAREA] SyncBridge iniciado (sincronización periódica)");
        sync_bridge.start_sync_loop().await
    });

    // ─── 7. Registro de auditoría de arranque ───
    audit.log_access(
        "windows_edge_agent@hcg.gob.mx",
        "AGENT_STARTED",
        None,
    ).await;

    // ─── 8. Graceful shutdown ante Ctrl+C o SIGTERM ───
    info!(target: "main", "Agente operativo. Esperando señales de terminación...");

    tokio::select! {
        _ = signal::ctrl_c() => {
            warn!(target: "main", "SIGINT recibido. Iniciando shutdown graceful...");
        }
        _ = async {
            // Windows no soporta SIGTERM nativamente en tokio::signal,
            // pero preparamos el hook para compatibilidad futura.
            #[cfg(unix)]
            {
                let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
                    .expect("No se pudo registrar SIGTERM");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            {
                futures::future::pending::<()>().await;
            }
        } => {
            warn!(target: "main", "SIGTERM recibido. Iniciando shutdown graceful...");
        }
    }

    // ─── 9. Flush final y terminación ───
    info!(target: "main", "Cerrando tareas...");

    // Abortar handles (las tareas tienen loops infinitos con intervalos)
    protocol_handle.abort();
    sync_handle.abort();

    // Esperar a que los canales de auditoría se vacíen
    sleep(Duration::from_secs(2)).await;

    audit.log_access(
        "windows_edge_agent@hcg.gob.mx",
        "AGENT_STOPPED",
        None,
    ).await;

    sleep(Duration::from_secs(1)).await;

    info!(target: "main", "Agente terminado. SQLite WAL flush completado.");
    Ok(())
}

// Helper para compatibilidad de sleep en main
use std::time::Duration;
use tokio::time::sleep;