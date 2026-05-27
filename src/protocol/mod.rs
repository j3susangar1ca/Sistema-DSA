//! src/protocol/mod.rs
//! Módulo: [SYNC-001] / [PROTO-001] Reverse Polling Engine v2.0 Mission-Critical
//! Cumplimiento: ISO/IEC 25010 (Reliability, Performance Efficiency, Portability)
//!
//! Execution Plane Edge. Cero puertos entrantes. Polling de baja latencia (1s)
//! sobre HTTPS outbound. Circuit breaker + backoff exponencial con jitter.
//! Integra FSM idempotente, SQLite WAL y auditoría no bloqueante.

use crate::config::EdgeConfig;
use crate::models::{CommandMessage, CommandStatus};
use crate::security::audit::AuditLogger;
use crate::storage::SQLiteManager;
use crate::utils::backoff::BackoffEngine;
use anyhow::{Context, Result};
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, instrument, warn};

// =============================================================================
// ESTADOS DEL CIRCUIT BREAKER
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    Closed,    // Operación normal
    Open,      // Fallos consecutivos excedieron umbral; rechaza polling
    HalfOpen,  // Ventana de prueba tras cooldown
}

struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    threshold: u32,
    cooldown: Duration,
    last_opened: Option<std::time::Instant>,
}

impl CircuitBreaker {
    fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            threshold,
            cooldown,
            last_opened: None,
        }
    }

    fn record_success(&mut self) {
        self.failure_count = 0;
        if self.state != CircuitState::Closed {
            info!(target: "protocol", "Circuit breaker cerrado (recuperación detectada)");
            self.state = CircuitState::Closed;
        }
    }

    fn record_failure(&mut self) -> CircuitState {
        self.failure_count += 1;
        if self.failure_count >= self.threshold && self.state == CircuitState::Closed {
            warn!(
                target: "protocol",
                "Circuit breaker ABIERTO tras {} fallos consecutivos",
                self.failure_count
            );
            self.state = CircuitState::Open;
            self.last_opened = Some(std::time::Instant::now());
        }
        self.state
    }

    fn try_half_open(&mut self) {
        if let Some(t) = self.last_opened {
            if t.elapsed() >= self.cooldown && self.state == CircuitState::Open {
                info!(target: "protocol", "Circuit breaker en prueba (Half-Open)");
                self.state = CircuitState::HalfOpen;
            }
        }
    }

    fn is_open(&self) -> bool {
        self.state == CircuitState::Open
    }

    fn is_half_open(&self) -> bool {
        self.state == CircuitState::HalfOpen
    }
}

// =============================================================================
// RESPUESTA DEL CONTROL PLANE
// =============================================================================

#[derive(Debug, Deserialize)]
struct ApiResponse {
    status: String,
    execution_status: String,
    #[serde(default)]
    command_state: String,
    #[serde(default)]
    response_payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CommandListWrapper {
    #[serde(default)]
    commands: Vec<<CommandMessage>,
}

// =============================================================================
// ERRORES DEL PROTOCOLO
// =============================================================================

#[derive(Debug, thiserror::Error)]
enum ProtocolError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Respuesta no exitosa del servidor: {0}")]
    ServerError(String),

    #[error("Acción desconocida: {0}")]
    UnknownAction(String),

    #[error("Módulo ejecutor no disponible: {0}")]
    ModuleNotAvailable(String),

    #[error("Error de persistencia local: {0}")]
    Storage(#[from] crate::storage::StorageError),
}

// =============================================================================
// MOTOR DE POLLING
// =============================================================================

pub struct ProtocolEngine {
    config: Arc<<EdgeConfig>,
    client: Client,
    sqlite: SQLiteManager,
    backoff: BackoffEngine,
    audit: AuditLogger,
    circuit: CircuitBreaker,
}

impl ProtocolEngine {
    /// Construye el motor con un cliente HTTP optimizado para Google Apps Script.
    pub fn new(config: EdgeConfig, sqlite: SQLiteManager, audit: AuditLogger) -> Self {
        let client = Client::builder()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() > 5 {
                    attempt.error("Exceso de redirecciones 302 en Control Plane")
                } else {
                    attempt.follow()
                }
            }))
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(10)
            .build()
            .expect("Construcción del cliente HTTP fallida");

        let backoff = BackoffEngine::new(config.backoff_base(), config.backoff_max(), config.backoff_max_retries);

        let circuit = CircuitBreaker::new(
            config.backoff_max_retries,
            Duration::from_secs(30),
        );

        Self {
            config: Arc::new(config),
            client,
            sqlite,
            backoff,
            audit,
            circuit,
        }
    }

    /// Bucle principal de polling. Nunca retorna (`-> !`).
    #[instrument(skip(self))]
    pub async fn start_polling_loop(mut self) -> ! {
        let mut ticker = interval(self.config.polling_interval());
        let mut heartbeat_counter: u8 = 0;

        info!(target: "protocol", "Motor de polling v2.0 iniciado. Intervalo: {:?}", self.config.polling_interval());

        loop {
            ticker.tick().await;

            // Evaluar circuit breaker
            if self.circuit.is_open() {
                self.circuit.try_half_open();
                if self.circuit.is_open() {
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }

            match self.tick().await {
                Ok(()) => {
                    self.circuit.record_success();
                    heartbeat_counter += 1;
                }
                Err(e) => {
                    let state = self.circuit.record_failure();
                    let delay = self.backoff.compute(self.circuit.failure_count);
                    error!(
                        target: "protocol",
                        "Tick fallido (estado circuito: {:?}): {}. Esperando {:?}",
                        state, e, delay
                    );
                    sleep(delay).await;
                    continue;
                }
            }

            // Heartbeat pasivo cada ~30 ciclos (≈30s) si no hay actividad
            if heartbeat_counter >= 30 {
                if let Err(e) = self.send_heartbeat().await {
                    debug!(target: "protocol", "Heartbeat opcional fallido: {}", e);
                }
                heartbeat_counter = 0;
            }
        }
    }

    /// Ciclo único de polling: solicita comandos, ejecuta, confirma.
    async fn tick(&self) -> Result<(), ProtocolError> {
        let poll_msg = CommandMessage::poll_commands("windows_edge_agent@hcg.gob.mx");

        let response = self
            .client
            .post(self.config.control_plane_url.clone())
            .json(&poll_msg)
            .send()
            .await
            .map_err(ProtocolError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProtocolError::ServerError(format!("HTTP {}: {}", status, body)));
        }

        let api_res: ApiResponse = response.json().await.map_err(ProtocolError::Http)?;

        // Si el servidor reporta comandos embebidos en response_payload, extraerlos
        let commands: Vec<<CommandMessage> = if let Ok(wrapper) =
            serde_json::from_value::<CommandListWrapper>(api_res.response_payload.clone())
        {
            wrapper.commands
        } else {
            // Simulación legacy: un solo comando embebido directamente
            vec![]
        };

        if commands.is_empty() {
            debug!(target: "protocol", "Cola vacía; sin comandos pendientes");
            return Ok(());
        }

        for cmd in commands {
            info!(
                target: "protocol",
                "Comando recibido: {} | acción: {}",
                cmd.command_id, cmd.action
            );

            if let Err(e) = self.process_command(&cmd).await {
                error!(
                    target: "protocol",
                    "Ejecución fallida para {}: {}",
                    cmd.command_id, e
                );
                self.ack_command(&cmd.command_id, CommandStatus::Failed, serde_json::json!({ "error": e.to_string() }))
                    .await
                    .ok();
            }
        }

        Ok(())
    }

    /// Ejecuta un comando individual y emite ACK.
    #[instrument(skip(self, cmd))]
    async fn process_command(&self, cmd: &CommandMessage) -> Result<(), ProtocolError> {
        // 1. Marcar IN_PROGRESS en SQLite para idempotencia at-least-once
        self.mark_command_status(&cmd.command_id, CommandStatus::InProgress, None)
            .await?;

        // 2. Dispatch según acción
        let result_payload = match cmd.action.as_str() {
            "SCRAPE_INTRANET_STATUS" => {
                // Delegación a PROXY-001 (intranet_scraping.rs — siguiente lote)
                Err(ProtocolError::ModuleNotAvailable(
                    "[PROXY-001] Intranet Scraping".into(),
                ))
            }
            "EXCEL_UPDATE_ROW" => {
                // Delegación a SYNC-001 / excel.rs
                Err(ProtocolError::ModuleNotAvailable(
                    "[SYNC-001] Excel Transactive Store (Update)".into(),
                ))
            }
            "EXCEL_APPEND_ROW" => {
                // Delegación a SYNC-001 / excel.rs
                Err(ProtocolError::ModuleNotAvailable(
                    "[SYNC-001] Excel Transactive Store (Append)".into(),
                ))
            }
            "LOCAL_FILE_SYNC" => {
                // Delegación a SYNC-001 / file watcher
                Err(ProtocolError::ModuleNotAvailable(
                    "[SYNC-001] SMB File Sync".into(),
                ))
            }
            "POLL_COMMANDS" | "ACK_COMMAND" | "HEARTBEAT_OK" => {
                // Acciones internas del protocolo; no requieren ejecución externa
                Ok(serde_json::json!({ "handled": "internal_protocol_action" }))
            }
            other => Err(ProtocolError::UnknownAction(other.to_string())),
        };

        // 3. Determinar estado final y ACK
        let (status, payload) = match result_payload {
            Ok(p) => (CommandStatus::Completed, p),
            Err(e) => (CommandStatus::Failed, serde_json::json!({ "error": e.to_string() })),
        };

        self.ack_command(&cmd.command_id, status, payload).await?;

        // 4. Auditoría
        self.audit
            .log_event(
                &cmd.command_id,
                &format!("COMMAND_{}", status),
                &cmd.requested_by,
                Some(serde_json::json!({ "action": &cmd.action })),
            )
            .await;

        Ok(())
    }

    /// Emite ACK de finalización al Control Plane.
    async fn ack_command(
        &self,
        command_id: &str,
        status: CommandStatus,
        payload: serde_json::Value,
    ) -> Result<(), ProtocolError> {
        let ack_msg = CommandMessage::ack(command_id.to_string(), status, payload);

        let res = self
            .client
            .post(self.config.control_plane_url.clone())
            .json(&ack_msg)
            .send()
            .await
            .map_err(ProtocolError::Http)?;

        if !res.status().is_success() {
            return Err(ProtocolError::ServerError(format!(
                "ACK fallido: HTTP {}",
                res.status()
            )));
        }

        // Persistir confirmación local
        self.mark_command_status(command_id, status, Some(ack_msg.timestamp))
            .await?;

        Ok(())
    }

    /// Heartbeat pasivo para mantener métricas de conectividad.
    async fn send_heartbeat(&self) -> Result<(), ProtocolError> {
        let hb = CommandMessage::heartbeat("windows_edge_agent@hcg.gob.mx");
        let res = self
            .client
            .post(self.config.control_plane_url.clone())
            .json(&hb)
            .send()
            .await
            .map_err(ProtocolError::Http)?;

        if res.status().is_success() {
            debug!(target: "protocol", "Heartbeat aceptado por Control Plane");
        }
        Ok(())
    }

    /// Actualiza el estado de un comando en la cola local SQLite.
    async fn mark_command_status(
        &self,
        command_id: &str,
        status: CommandStatus,
        completed_at: Option<<chrono::DateTime<<chrono::Utc>>,
    ) -> Result<(), ProtocolError> {
        let id = command_id.to_string();
        let st = status.to_string();
        let ts = completed_at.map(|t| t.to_rfc3339());

        self.sqlite
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO command_queue (command_id, action, timestamp, requested_by, execution_status, completed_at)
                     VALUES (?1, 'INTERNAL', datetime('now'), 'edge_agent', ?2, ?3)
                     ON CONFLICT(command_id) DO UPDATE SET
                        execution_status = excluded.execution_status,
                        completed_at = excluded.completed_at",
                    rusqlite::params![id, st, ts],
                )
            })
            .await
            .map_err(ProtocolError::Storage)?;

        Ok(())
    }
}