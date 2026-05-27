//! src/models/mod.rs
//! Módulo: [LEDGER-001] / [MODEL-001] Schema Canónico y Tipos de Dominio
//! Cumplimiento: ISO/IEC 25010 (Maintainability, Reliability)
//!
//! Declaración formal del modelo de datos canónico `FondoRevolventeLedger`
//! y los tipos de dominio del protocolo Cloud-Edge. Fuente única de verdad
//! para SQLite, BigQuery, Excel y JSON.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

// Re-exportar submódulos de dominio cuando se materialicen.
// pub mod fsm;

// =============================================================================
// BLOQUE A: Estatus Transaccional del Trámite
// =============================================================================

/// Estado civil del trámite de fondo revolvente.
/// En Rust se conserva PascalCase por idioma del lenguaje.
/// En SQL/JSON se serializa como `UPPER_SNAKE_CASE` ([ID-REQ-LEDGER-02]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EstatusTramite {
    Cotizacion,
    RecursosFinancieros,
    AutorizadoCaa,
    AutorizadoSub,
    Cancelado,
    Entregado,
}

impl fmt::Display for EstatusTramite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Cotizacion => "COTIZACION",
            Self::RecursosFinancieros => "RECURSOS_FINANCIEROS",
            Self::AutorizadoCaa => "AUTORIZADO_CAA",
            Self::AutorizadoSub => "AUTORIZADO_SUB",
            Self::Cancelado => "CANCELADO",
            Self::Entregado => "ENTREGADO",
        };
        write!(f, "{}", s)
    }
}

// =============================================================================
// BLOQUE B: Snapshot Financiero (Sub-struct Anidado)
// =============================================================================

/// Representación estructurada de los importes del hito Pedido (Bloque 4).
/// Al serializar hacia SQL/BigQuery se despliega en 4 columnas planas.
/// Si es `None`, las 4 columnas se insertan como `NULL` ([ID-REQ-LEDGER-04]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancieroSnapshot {
    pub precio_unitario: f64,
    pub monto_subtotal: f64,
    pub monto_iva: f64,
    pub monto_total_con_iva: f64,
}

// =============================================================================
// BLOQUE C: Schema Canónico — FondoRevolventeLedger
// =============================================================================

/// Entidad canónica compuesta por 5 bloques de datos correspondientes a los
/// hitos operativos del ciclo de vida del fondo revolvente.
///
/// Invariantes de Type Safety ([ID-REQ-LEDGER-04]):
/// - Campos numéricos: `f64` en Rust → `NUMERIC` / `DECIMAL(18,4)` en SQL.
/// - Fechas: `NaiveDate` (sin timezone implícita).
/// - Opcionales: `Option<T>` estricto. Cero valores centinela (`""`, `0`, `1900-01-01`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FondoRevolventeLedger {
    // ─── Bloque 1: Ingesta e Identificación ───
    pub folio_dsa: String,
    pub tipo_tramite: String,
    pub fecha_recepcion: NaiveDate,
    pub servicio_solicitante: String,
    pub oficio_solicitud: String,
    pub codigo: String,
    pub descripcion: String,
    pub cantidad_solicitada: f64,
    pub unidad_medida: String,
    pub partida_especifica: String,

    // ─── Bloque 2: Control y Operación Interna ───
    pub usuario_asignado: String,
    pub fecha_inicio_cotizacion: Option<NaiveDate>,
    pub estatus_tramite: EstatusTramite,
    pub observaciones: Option<String>,

    // ─── Bloque 3: Validación Presupuestal e Institucional (Hito SUPRE + CAA) ───
    pub folio_supre: Option<String>,
    pub fecha_supre: Option<NaiveDate>,
    pub paquete_envio_caa: Option<i64>,
    pub fecha_recibido_caa: Option<NaiveDate>,
    pub fecha_autorizacion_caa: Option<NaiveDate>,
    pub folio_autorizacion_caa: Option<String>,

    // ─── Bloque 4: Adjudicación e Importes Financieros (Hito Pedido) ───
    pub financieros: Option<FinancieroSnapshot>,
    pub cantidad_pedido: Option<f64>,
    pub numero_pedido: Option<String>,
    pub fecha_pedido: Option<NaiveDate>,
    pub proveedor_rfc: Option<String>,

    // ─── Bloque 5: Logística y Cierre Fiscal (Hito Pasivo/Pago) ───
    pub estatus_entrega: Option<String>,
    pub fecha_entrega_almacen: Option<NaiveDate>,
    pub numero_factura: Option<String>,
    pub fecha_factura: Option<NaiveDate>,
    pub fecha_envio_xml_rf: Option<NaiveDate>,
    pub fecha_pago: Option<NaiveDate>,
    pub fecha_complemento_pago_rf: Option<NaiveDate>,

    // ─── Metadatos de Auditoría Transversal ───
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Estado de sincronización con BigQuery (Edge-specific).
    pub sync_status: SyncStatus,
}

impl FondoRevolventeLedger {
    /// Constructor mínimo para un expediente recién ingresado (Bloques 1 y 2 poblados).
    pub fn new(
        folio_dsa: String,
        fecha_recepcion: NaiveDate,
        servicio_solicitante: String,
        oficio_solicitud: String,
        codigo: String,
        descripcion: String,
        cantidad_solicitada: f64,
        unidad_medida: String,
        partida_especifica: String,
        usuario_asignado: String,
        estatus_tramite: EstatusTramite,
    ) -> Self {
        let now = Utc::now();
        Self {
            folio_dsa,
            tipo_tramite: "COMPRA POR FONDO".into(),
            fecha_recepcion,
            servicio_solicitante,
            oficio_solicitud,
            codigo,
            descripcion,
            cantidad_solicitada,
            unidad_medida,
            partida_especifica,
            usuario_asignado,
            fecha_inicio_cotizacion: None,
            estatus_tramite,
            observaciones: None,
            folio_supre: None,
            fecha_supre: None,
            paquete_envio_caa: None,
            fecha_recibido_caa: None,
            fecha_autorizacion_caa: None,
            folio_autorizacion_caa: None,
            financieros: None,
            cantidad_pedido: None,
            numero_pedido: None,
            fecha_pedido: None,
            proveedor_rfc: None,
            estatus_entrega: None,
            fecha_entrega_almacen: None,
            numero_factura: None,
            fecha_factura: None,
            fecha_envio_xml_rf: None,
            fecha_pago: None,
            fecha_complemento_pago_rf: None,
            created_at: now,
            updated_at: now,
            sync_status: SyncStatus::Pending,
        }
    }

    /// Clave primaria compuesta canónica: `(folio_dsa, codigo)`.
    pub fn pk(&self) -> (&str, &str) {
        (&self.folio_dsa, &self.codigo)
    }

    /// Verifica si el registro tiene datos financieros poblados (Bloque 4).
    pub fn has_financieros(&self) -> bool {
        self.financieros.is_some()
    }
}

// =============================================================================
// BLOQUE D: Tipos de Dominio del Protocolo Cloud-Edge
// =============================================================================

/// Estado de ejecución de un comando en la cola inversa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for CommandStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "PENDING"),
            Self::InProgress => write!(f, "IN_PROGRESS"),
            Self::Completed => write!(f, "COMPLETED"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}

/// Estado de sincronización de un registro hacia BigQuery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncStatus {
    Pending,
    Uploading,
    Synced,
    Failed,
    Blocked,
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "PENDING"),
            Self::Uploading => write!(f, "UPLOADING"),
            Self::Synced => write!(f, "SYNCED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Blocked => write!(f, "SYNC_BLOCKED"),
        }
    }
}

/// Mensaje de comando del protocolo inverso Cloud-Edge ([SYNC-001]).
/// Representa la unidad atómica de trabajo despachada desde el Control Plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandMessage {
    pub command_id: String,
    pub action: String,
    pub timestamp: DateTime<Utc>,
    pub requested_by: String,
    pub execution_status: CommandStatus,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub response_payload: Option<serde_json::Value>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl CommandMessage {
    /// Identificador único con prefijo canónico.
    pub fn generate_id() -> String {
        format!("cmd_dsa_{}", Utc::now().timestamp_millis())
    }

    /// Constructor de polling heartbeat.
    pub fn heartbeat(requested_by: &str) -> Self {
        Self {
            command_id: Self::generate_id(),
            action: "HEARTBEAT_OK".into(),
            timestamp: Utc::now(),
            requested_by: requested_by.into(),
            execution_status: CommandStatus::Pending,
            payload: serde_json::Value::Null,
            response_payload: None,
            completed_at: None,
        }
    }

    /// Constructor de solicitud de comandos pendientes.
    pub fn poll_commands(requested_by: &str) -> Self {
        Self {
            command_id: Self::generate_id(),
            action: "POLL_COMMANDS".into(),
            timestamp: Utc::now(),
            requested_by: requested_by.into(),
            execution_status: CommandStatus::Pending,
            payload: serde_json::Value::Null,
            response_payload: None,
            completed_at: None,
        }
    }

    /// Constructor de ACK post-ejecución.
    pub fn ack(command_id: String, status: CommandStatus, payload: serde_json::Value) -> Self {
        Self {
            command_id,
            action: "ACK_COMMAND".into(),
            timestamp: Utc::now(),
            requested_by: "windows_edge_agent@hcg.gob.mx".into(),
            execution_status: status,
            payload,
            response_payload: None,
            completed_at: Some(Utc::now()),
        }
    }
}

// =============================================================================
// BLOQUE E: Registro Intermedio WAL (Write-Ahead Log)
// =============================================================================

/// Entidad de buffer local antes de la carga batch en BigQuery.
/// Garantiza cero pérdida de datos ante caídas de red ([ID-REQ-SYNC-01·P]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WALRecord {
    pub record_id: String,
    pub table_name: String,
    pub payload: serde_json::Value,
    pub status: SyncStatus,
    pub created_at: DateTime<Utc>,
    pub synced_at: Option<DateTime<Utc>>,
}

impl WALRecord {
    pub fn new(table_name: String, payload: serde_json::Value) -> Self {
        Self {
            record_id: format!("wal_{}", Utc::now().timestamp_millis()),
            table_name,
            payload,
            status: SyncStatus::Pending,
            created_at: Utc::now(),
            synced_at: None,
        }
    }
}