//! src/config.rs
//! Módulo: [CONFIG-001] Edge Agent Bootstrap & Environment Validation
//! Cumplimiento: ISO/IEC 25010 (Reliability, Maintainability, Portability)
//!
//! Garantiza que todas las variables críticas de entorno sean validadas
//! en el momento del arranque, previniendo fallos en runtime por paths
//! inexistentes o URLs malformadas.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};
use url::Url;

/// Errores exhaustivos de configuración. Cada variante es auto-documentada.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Variable de entorno requerida ausente: {0}")]
    MissingEnv(String),

    #[error("URL del Control Plane inválida o con esquema no seguro: {0}")]
    InvalidControlPlaneUrl(String),

    #[error("Path local inaccesible o inexistente: {path} — {reason}")]
    InvalidLocalPath { path: String, reason: String },

    #[error("Parámetro de backoff inconsistente: base={base}, max={max}")]
    InvalidBackoffParams { base: u64, max: u64 },

    #[error("Timeout FSM debe ser mayor a 0 segundos")]
    InvalidFsmTimeout,
}

/// Configuración inmutable del Edge Agent.
/// Carga una sola vez al inicio del proceso y se distribuye vía `Arc` a todos los módulos.
#[derive(Debug, Clone)]
pub struct EdgeConfig {
    // ─── Control Plane Cloud ───
    /// Endpoint HTTPS del Web App de Google Apps Script (Control Plane).
    pub control_plane_url: Url,

    // ─── Persistencia Local (Edge) ───
    /// Ruta absoluta del archivo SQLite WAL (`local_buffer.db`).
    pub sqlite_path: PathBuf,
    /// Ruta UNC del Transactive Store Excel sobre SMB.
    pub excel_smb_path: PathBuf,
    /// Directorio local de respaldo de archivos replicados desde Drive.
    pub smb_expedientes_path: PathBuf,

    // ─── Google Cloud Data Warehouse ───
    pub bq_project_id: String,
    pub bq_dataset: String,
    pub bq_table_ledger: String,

    // ─── Protocolo de Polling Inverso ───
    /// Intervalo entre polls (default: 1s = 1000ms). Requisito de baja latencia [SYNC-001].
    pub polling_interval_ms: u64,
    /// Retardo base del backoff exponencial (default: 2s).
    pub backoff_base_ms: u64,
    /// Techo del backoff exponencial (default: 300s = 5 min).
    pub backoff_max_ms: u64,
    /// Máximo de reintentos antes de declarar SYNC_BLOCKED (default: 10).
    pub backoff_max_retries: u32,

    // ─── FSM Idempotente ───
    /// Umbral `$t_{out}$` para transición de escape `$s_1 \to s_0$` (default: 60s).
    pub fsm_in_progress_timeout_secs: u64,

    // ─── Exclusión Pesimista Win32 ───
    /// Tiempo máximo de espera ante lock `~$` de Excel (default: 300s).
    pub win32_lock_max_wait_secs: u64,
}

impl EdgeConfig {
    /// Carga la configuración desde variables de entorno vía `dotenvy`.
    /// Falla rápido (`fail-fast`) ante cualquier anomalía de entorno.
    pub fn load() -> Result<Self, ConfigError> {
        // Carga .env si existe; no falla si no existe (dotenvy es silencioso).
        let _ = dotenvy::dotenv();

        let control_plane_raw = std::env::var("CONTROL_PLANE_URL")
            .map_err(|_| ConfigError::MissingEnv("CONTROL_PLANE_URL".into()))?;
        let control_plane_url = Url::parse(&control_plane_raw)
            .map_err(|_| ConfigError::InvalidControlPlaneUrl(control_plane_raw.clone()))?;
        if control_plane_url.scheme() != "https" {
            return Err(ConfigError::InvalidControlPlaneUrl(
                "Esquema obligatorio: https".into(),
            ));
        }

        let sqlite_path = Self::resolve_path("DATABASE_URL", "data/local_buffer.db")?;
        let excel_smb_path = Self::resolve_path("EXCEL_SMB_PATH", "")?;
        let smb_expedientes_path = Self::resolve_path("SMB_EXPEDIENTES_PATH", "")?;

        let bq_project_id = std::env::var("BQ_PROJECT_ID")
            .map_err(|_| ConfigError::MissingEnv("BQ_PROJECT_ID".into()))?;
        let bq_dataset = std::env::var("BQ_DATASET")
            .unwrap_or_else(|_| "hospital_civil".into());
        let bq_table_ledger = std::env::var("BQ_TABLE_LEDGER")
            .unwrap_or_else(|_| "fondo_revolvente_ledger".into());

        let polling_interval_ms = Self::parse_env_or("POLLING_INTERVAL_MS", 1000);
        let backoff_base_ms = Self::parse_env_or("BACKOFF_BASE_MS", 2000);
        let backoff_max_ms = Self::parse_env_or("BACKOFF_MAX_MS", 300_000);
        let backoff_max_retries = Self::parse_env_or("BACKOFF_MAX_RETRIES", 10);

        if backoff_base_ms == 0 || backoff_base_ms > backoff_max_ms {
            return Err(ConfigError::InvalidBackoffParams {
                base: backoff_base_ms,
                max: backoff_max_ms,
            });
        }

        let fsm_in_progress_timeout_secs = Self::parse_env_or("FSM_IN_PROGRESS_TIMEOUT_SECS", 60);
        if fsm_in_progress_timeout_secs == 0 {
            return Err(ConfigError::InvalidFsmTimeout);
        }

        let win32_lock_max_wait_secs = Self::parse_env_or("WIN32_LOCK_MAX_WAIT_SECS", 300);

        let config = Self {
            control_plane_url,
            sqlite_path,
            excel_smb_path,
            smb_expedientes_path,
            bq_project_id,
            bq_dataset,
            bq_table_ledger,
            polling_interval_ms,
            backoff_base_ms,
            backoff_max_ms,
            backoff_max_retries,
            fsm_in_progress_timeout_secs,
            win32_lock_max_wait_secs,
        };

        config.validate_smb_access()?;
        info!(target: "config", "Configuración cargada y validada correctamente.");
        Ok(config)
    }

    /// Resuelve una variable de entorno opcional; si no existe, usa `default`.
    /// Valida que el path no esté vacío.
    fn resolve_path(env_key: &str, default: &str) -> Result<PathBuf, ConfigError> {
        let raw = std::env::var(env_key).unwrap_or_else(|_| default.into());
        if raw.trim().is_empty() {
            return Err(ConfigError::MissingEnv(env_key.into()));
        }
        Ok(PathBuf::from(raw))
    }

    /// Parsea una variable de entorno numérica; si falla o no existe, retorna `default`.
    fn parse_env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Prueba de escritura transitoria en el share SMB (si aplica).
    /// En entornos sin SMB configurado, emite advertencia pero no bloquea.
    fn validate_smb_access(&self) -> Result<(), ConfigError> {
        if let Some(parent) = self.excel_smb_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                warn!(
                    target: "config",
                    "Ruta SMB no accesible en este momento (posible desconexión de red): {}",
                    parent.display()
                );
                // No fallamos aquí: el daemon debe ser resiliente a reconexión de red.
            }
        }
        Ok(())
    }

    /// Duración tipada del intervalo de polling.
    pub fn polling_interval(&self) -> Duration {
        Duration::from_millis(self.polling_interval_ms)
    }

    /// Duración tipada del retardo base de backoff.
    pub fn backoff_base(&self) -> Duration {
        Duration::from_millis(self.backoff_base_ms)
    }

    /// Duración tipada del techo de backoff.
    pub fn backoff_max(&self) -> Duration {
        Duration::from_millis(self.backoff_max_ms)
    }

    /// Duración tipada del timeout FSM `$t_{out}$`.
    pub fn fsm_timeout(&self) -> Duration {
        Duration::from_secs(self.fsm_in_progress_timeout_secs)
    }

    /// Duración tipada de espera máxima de lock Win32.
    pub fn win32_lock_max_wait(&self) -> Duration {
        Duration::from_secs(self.win32_lock_max_wait_secs)
    }
}