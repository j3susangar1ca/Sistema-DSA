//! src/security/audit.rs
//! Módulo: [AUTH-001] / [AUDIT-001] Zero-Blocking Audit Logger
//! Cumplimiento: ISO/IEC 25010 (Reliability, Security)
//!
//! Canal de productor-consumidor asíncrono (mpsc) que garantiza que la
//! escritura de auditoría nunca bloquee el hilo operativo principal.
//! Persiste en SQLite local (`access_audit_log` y `expedition_events`).

use crate::storage::SQLiteManager;
use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};

/// Registro interno de auditoría transportado por el canal.
#[derive(Debug)]
enum AuditRecord {
    Access {
        email: String,
        result: String,
        client_ip: Option<String>,
    },
    Event {
        expedition_id: String,
        event_type: String,
        actor: String,
        payload: Option<Value>,
    },
}

/// Logger de auditoría no bloqueante.
///
/// Uso: `logger.log_access(email, "GRANTED", Some("10.0.0.5")).await;`
#[derive(Debug, Clone)]
pub struct AuditLogger {
    tx: mpsc::Sender<<AuditRecord>,
}

impl AuditLogger {
    /// Inicializa el consumidor de auditoría en una tarea Tokio dedicada.
    pub fn new(sqlite: SQLiteManager) -> Self {
        let (tx, mut rx) = mpsc::channel::<AuditRecord>(256);

        // Consumidor asíncrono: aísla la I/O de SQLite del flujo operativo
        tokio::spawn(async move {
            while let Some(record) = rx.recv().await {
                let sqlite = sqlite.clone();
                match record {
                    AuditRecord::Access {
                        email,
                        result,
                        client_ip,
                    } => {
                        let ts = Utc::now().to_rfc3339();
                        let id = format!("audit_{}", Utc::now().timestamp_millis());
                        let ip = client_ip.unwrap_or_default();

                        let res = sqlite
                            .execute(move |conn| {
                                conn.execute(
                                    "INSERT INTO access_audit_log (id, email, result, client_ip, timestamp)
                                     VALUES (?1, ?2, ?3, ?4, ?5)",
                                    params![&id, &email, &result, &ip, &ts],
                                )
                            })
                            .await;

                        if let Err(e) = res {
                            error!(target: "audit", "Fallo al persistir access_audit_log: {}", e);
                        } else {
                            info!(target: "audit", "Acceso registrado: {} -> {}", email, result);
                        }
                    }

                    AuditRecord::Event {
                        expedition_id,
                        event_type,
                        actor,
                        payload,
                    } => {
                        let ts = Utc::now().to_rfc3339();
                        let id = format!("evt_{}", Utc::now().timestamp_millis());
                        let payload_json = payload.map(|v| v.to_string()).unwrap_or_default();

                        let res = sqlite
                            .execute(move |conn| {
                                conn.execute(
                                    "INSERT INTO expedition_events (id, expedition_id, event_type, actor, payload, timestamp)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                    params![
                                        &id,
                                        &expedition_id,
                                        &event_type,
                                        &actor,
                                        &payload_json,
                                        &ts
                                    ],
                                )
                            })
                            .await;

                        if let Err(e) = res {
                            error!(target: "audit", "Fallo al persistir expedition_event: {}", e);
                        }
                    }
                }
            }
            warn!(target: "audit", "Canal de auditoría cerrado; consumidor terminado.");
        });

        Self { tx }
    }

    /// Registra un intento de acceso (exitoso o denegado) de forma asíncrona.
    #[instrument(skip(self))]
    pub async fn log_access(&self, email: &str, result: &str, client_ip: Option<&str>) {
        let record = AuditRecord::Access {
            email: email.to_string(),
            result: result.to_string(),
            client_ip: client_ip.map(|s| s.to_string()),
        };
        if let Err(e) = self.tx.send(record).await {
            warn!(target: "audit", "Canal saturado, registro de acceso descartado: {}", e);
        }
    }

    /// Registra un evento de dominio en la bitácora append-only del expediente.
    #[instrument(skip(self, payload))]
    pub async fn log_event(
        &self,
        expedition_id: &str,
        event_type: &str,
        actor: &str,
        payload: Option<Value>,
    ) {
        let record = AuditRecord::Event {
            expedition_id: expedition_id.to_string(),
            event_type: event_type.to_string(),
            actor: actor.to_string(),
            payload,
        };
        if let Err(e) = self.tx.send(record).await {
            warn!(target: "audit", "Canal saturado, evento descartado: {}", e);
        }
    }
}