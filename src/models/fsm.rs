//! src/models/fsm.rs
//! Módulo: [EXP-001] Expedition State Machine — Autómata Híbrido Robusto
//! Cumplimiento: ISO/IEC 25010 (Reliability, Maintainability)
//!
//! Implementación matemática de la matriz de transiciones extendida del
//! ciclo de vida del expediente, con transición de escape por timeout
//! (s1 → s0) y eventos log-only.

use std::fmt;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, warn};

// =============================================================================
// ESTADOS DEL EXPEDIENTE (ExpeditionStatusEnum)
// =============================================================================

/// Estados de la FSM extendida del expediente.
/// 20 variantes documentadas en [EXP-001] Sección 3.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpeditionStatus {
    Initiated,
    Scanning,
    DocsCaptured,
    InferencePending,
    Validated,
    CatalogChecked,
    PendingProcurementVerification,
    ProcedenciaAprobada,
    AsignacionProveedores,
    EsperaCotizaciones,
    CuadroComparativoConsolidado,
    AdjudicacionSugerida,
    EnviadoRecursosFinancieros,
    AutorizadoSubdireccion,
    Completed,
    CotizacionesVencidas,
    // Estados terminales de rechazo
    RejectedValidationFailed,
    RejectedCatalogInactive,
    RejectedProcurementDenied,
    // Deprecados (rechazados en transición)
    CotizacionesRecibidas,
    AllQuotationsReceived,
}

impl fmt::Display for ExpeditionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Initiated => "INITIATED",
            Self::Scanning => "SCANNING",
            Self::DocsCaptured => "DOCS_CAPTURED",
            Self::InferencePending => "INFERENCE_PENDING",
            Self::Validated => "VALIDATED",
            Self::CatalogChecked => "CATALOG_CHECKED",
            Self::PendingProcurementVerification => "PENDING_PROCUREMENT_VERIFICATION",
            Self::ProcedenciaAprobada => "PROCEDENCIA_APROBADA",
            Self::AsignacionProveedores => "ASIGNACION_PROVEEDORES",
            Self::EsperaCotizaciones => "ESPERA_COTIZACIONES",
            Self::CuadroComparativoConsolidado => "CUADRO_COMPARATIVO_CONSOLIDADO",
            Self::AdjudicacionSugerida => "ADJUDICACION_SUGERIDA",
            Self::EnviadoRecursosFinancieros => "ENVIADO_RECURSOS_FINANCIEROS",
            Self::AutorizadoSubdireccion => "AUTORIZADO_SUBDIRECCION",
            Self::Completed => "COMPLETED",
            Self::CotizacionesVencidas => "COTIZACIONES_VENCIDAS",
            Self::RejectedValidationFailed => "REJECTED_VALIDATION_FAILED",
            Self::RejectedCatalogInactive => "REJECTED_CATALOG_INACTIVE",
            Self::RejectedProcurementDenied => "REJECTED_PROCUREMENT_DENIED",
            Self::CotizacionesRecibidas => "COTIZACIONES_RECIBIDAS",
            Self::AllQuotationsReceived => "ALL_QUOTATIONS_RECEIVED",
        };
        write!(f, "{}", s)
    }
}

impl ExpeditionStatus {
    /// Determina si el estado es terminal (sin transiciones de salida permitidas).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::CotizacionesVencidas
                | Self::RejectedValidationFailed
                | Self::RejectedCatalogInactive
                | Self::RejectedProcurementDenied
        )
    }

    /// Estados que admiten transiciones de escape por timeout.
    pub fn is_in_progress(self) -> bool {
        matches!(self, Self::Scanning | Self::InferencePending | Self::EsperaCotizaciones)
    }
}

// =============================================================================
// EVENTOS DISPARADORES (EventTypeEnum)
// =============================================================================

/// Eventos que actúan como entradas al autómata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    Created,
    ScanStarted,
    ScanCompleted,
    DriveUploaded,
    InferenceStarted,
    InferenceCompleted,
    ValidationPassed,
    ValidationFailed,
    CatalogValid,
    CatalogInvalid,
    EmailSent,
    ResponseReceived,
    ProcedenciaAprobadaEvent,
    ProcedenciaDenegada,
    UserCommit,
    StateTransition,
    SuppliersAssigned,
    QuotationsDispatched,
    QuotationReceived,
    AllQuotationsReceived,
    QuotationDeadlineExpired,
    UserCommitPartial,
    QuotationValidated,
    QuotationRejectedNormative,
    ComparativeMatrixConsolidated,
    AwardRecommendationGenerated,
    AwardConfirmed,
    DraftsCreated,
    AutorizacionSubdireccionReceived,
    SupreAssigned,
    CaaPackageSent,
    CaaAuthorized,
    OrderIssued,
    DeliveryReceived,
    InvoiceRegistered,
    PaymentCompleted,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Created => "CREATED",
            Self::ScanStarted => "SCAN_STARTED",
            Self::ScanCompleted => "SCAN_COMPLETED",
            Self::DriveUploaded => "DRIVE_UPLOADED",
            Self::InferenceStarted => "INFERENCE_STARTED",
            Self::InferenceCompleted => "INFERENCE_COMPLETED",
            Self::ValidationPassed => "VALIDATION_PASSED",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::CatalogValid => "CATALOG_VALID",
            Self::CatalogInvalid => "CATALOG_INVALID",
            Self::EmailSent => "EMAIL_SENT",
            Self::ResponseReceived => "RESPONSE_RECEIVED",
            Self::ProcedenciaAprobadaEvent => "PROCEDENCIA_APROBADA",
            Self::ProcedenciaDenegada => "PROCEDENCIA_DENEGADA",
            Self::UserCommit => "USER_COMMIT",
            Self::StateTransition => "STATE_TRANSITION",
            Self::SuppliersAssigned => "SUPPLIERS_ASSIGNED",
            Self::QuotationsDispatched => "QUOTATIONS_DISPATCHED",
            Self::QuotationReceived => "QUOTATION_RECEIVED",
            Self::AllQuotationsReceived => "ALL_QUOTATIONS_RECEIVED",
            Self::QuotationDeadlineExpired => "QUOTATION_DEADLINE_EXPIRED",
            Self::UserCommitPartial => "USER_COMMIT_PARTIAL",
            Self::QuotationValidated => "QUOTATION_VALIDATED",
            Self::QuotationRejectedNormative => "QUOTATION_REJECTED_NORMATIVE",
            Self::ComparativeMatrixConsolidated => "COMPARATIVE_MATRIX_CONSOLIDATED",
            Self::AwardRecommendationGenerated => "AWARD_RECOMMENDATION_GENERATED",
            Self::AwardConfirmed => "AWARD_CONFIRMED",
            Self::DraftsCreated => "DRAFTS_CREATED",
            Self::AutorizacionSubdireccionReceived => "AUTORIZACION_SUBDIRECCION_RECEIVED",
            Self::SupreAssigned => "SUPRE_ASSIGNED",
            Self::CaaPackageSent => "CAA_PACKAGE_SENT",
            Self::CaaAuthorized => "CAA_AUTHORIZED",
            Self::OrderIssued => "ORDER_ISSUED",
            Self::DeliveryReceived => "DELIVERY_RECEIVED",
            Self::InvoiceRegistered => "INVOICE_REGISTERED",
            Self::PaymentCompleted => "PAYMENT_COMPLETED",
        };
        write!(f, "{}", s)
    }
}

// =============================================================================
// ERRORES DE TRANSICIÓN
// =============================================================================

#[derive(Debug, Error)]
pub enum FSMTransitionError {
    #[error("Transición ilegal: de {from} con evento {event}")]
    IllegalTransition { from: ExpeditionStatus, event: EventType },

    #[error("Estado terminal {0} no admite transiciones de salida")]
    TerminalState(ExpeditionStatus),

    #[error("Evento deprecado rechazado: {0}")]
    DeprecatedEvent(EventType),

    #[error("Transición de escape forzada por timeout (s1 -> s0)")]
    EscapeTimeout,
}

// =============================================================================
// CONTEXTO DEL AUTÓMATA
// =============================================================================

/// Estado mutable del autómata por expediente.
/// Mantiene el timestamp de entrada en estados in-progress para evaluar
/// la transición de escape `$s_1 \to s_0$`.
#[derive(Debug, Clone)]
pub struct FSMContext {
    pub in_progress_since: Option<<Instant>,
    pub retry_count: u32,
}

impl FSMContext {
    pub fn new() -> Self {
        Self {
            in_progress_since: None,
            retry_count: 0,
        }
    }

    /// Registra la entrada a un estado in-progress.
    pub fn enter_in_progress(&mut self) {
        self.in_progress_since = Some(Instant::now());
        self.retry_count = 0;
    }

    /// Verifica si se ha excedido el timeout `$t_{out}$`.
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.in_progress_since
            .map(|t| t.elapsed() > timeout)
            .unwrap_or(false)
    }

    /// Incrementa contador de reintento (usado en backoff de sync).
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

impl Default for FSMContext {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// MOTOR FSM
// =============================================================================

/// Motor de transiciones determinista con transición de escape temporal.
pub struct FSMEngine;

impl FSMEngine {
    /// Evalúa la transición de escape ANTES de procesar el evento.
    /// Si `current == IN_PROGRESS` y `ctx.is_timed_out(timeout)`, fuerza
    /// la reversión a `Pending` (o su equivalente semántico).
    pub fn check_escape(
        current: ExpeditionStatus,
        ctx: &mut FSMContext,
        timeout: Duration,
    ) -> Option<<ExpeditionStatus> {
        if current.is_in_progress() && ctx.is_timed_out(timeout) {
            warn!(
                target: "fsm",
                "Transición de escape activada: {} -> PENDING (timeout {:?})",
                current, timeout
            );
            ctx.in_progress_since = None;
            ctx.retry_count += 1;
            // El estado de escape depende del estado actual:
            return Some(match current {
                ExpeditionStatus::Scanning => ExpeditionStatus::Initiated,
                ExpeditionStatus::InferencePending => ExpeditionStatus::DocsCaptured,
                ExpeditionStatus::EsperaCotizaciones => ExpeditionStatus::AsignacionProveedores,
                _ => ExpeditionStatus::Initiated,
            });
        }
        None
    }

    /// Transición principal del autómata.
    ///
    /// Invariantes:
    /// - Ningún estado terminal acepta transiciones de salida.
    /// - `QuotationReceived` es log-only (no muta estado).
    /// - Eventos deprecados son rechazados.
    /// - `ValidationFailed` / `CatalogInvalid` transicionan desde cualquier activo.
    pub fn transition(
        current: ExpeditionStatus,
        event: EventType,
        ctx: &mut FSMContext,
    ) -> Result<<ExpeditionStatus, FSMTransitionError> {
        // 1. Guardas de estado terminal
        if current.is_terminal() {
            return Err(FSMTransitionError::TerminalState(current));
        }

        // 2. Guardas de eventos deprecados
        if matches!(
            event,
            EventType::CotizacionesRecibidas | EventType::AllQuotationsReceived
        ) {
            return Err(FSMTransitionError::DeprecatedEvent(event));
        }

        // 3. Eventos globales de rechazo (aplicables desde cualquier estado activo)
        match event {
            EventType::ValidationFailed => {
                return Ok(ExpeditionStatus::RejectedValidationFailed);
            }
            EventType::CatalogInvalid => {
                return Ok(ExpeditionStatus::RejectedCatalogInactive);
            }
            _ => {}
        }

        // 4. Matriz de transiciones extendida [EXP-001] Sección 3.1.1.1
        let next = match (current, event) {
            // Ingesta y Captura
            (ExpeditionStatus::Initiated, EventType::ScanStarted) => {
                ctx.enter_in_progress();
                ExpeditionStatus::Scanning
            }
            (ExpeditionStatus::Scanning, EventType::DriveUploaded) => {
                ctx.in_progress_since = None;
                ExpeditionStatus::DocsCaptured
            }

            // Inferencia y Validación
            (ExpeditionStatus::DocsCaptured, EventType::InferenceStarted) => {
                ctx.enter_in_progress();
                ExpeditionStatus::InferencePending
            }
            (ExpeditionStatus::InferencePending, EventType::ValidationPassed) => {
                ctx.in_progress_since = None;
                ExpeditionStatus::Validated
            }

            // Catálogo
            (ExpeditionStatus::Validated, EventType::CatalogValid) => {
                ExpeditionStatus::CatalogChecked
            }

            // Verificación Institucional (CAA / SUPRE)
            (ExpeditionStatus::CatalogChecked, EventType::EmailSent) => {
                ExpeditionStatus::PendingProcurementVerification
            }
            (
                ExpeditionStatus::PendingProcurementVerification,
                EventType::ResponseReceived,
            ) => ExpeditionStatus::ProcedenciaAprobada,
            (
                ExpeditionStatus::PendingProcurementVerification,
                EventType::ProcedenciaDenegada,
            ) => ExpeditionStatus::RejectedProcurementDenied,

            // Asignación y Cotización
            (
                ExpeditionStatus::ProcedenciaAprobada,
                EventType::SuppliersAssigned,
            ) => ExpeditionStatus::AsignacionProveedores,
            (
                ExpeditionStatus::AsignacionProveedores,
                EventType::QuotationsDispatched,
            ) => {
                ctx.enter_in_progress();
                ExpeditionStatus::EsperaCotizaciones
            }

            // Espera de Cotizaciones — Estados complejos
            (ExpeditionStatus::EsperaCotizaciones, EventType::QuotationReceived) => {
                // Evento LOG-ONLY: no muta estado, solo registra evento
                debug!(target: "fsm", "Evento log-only: QUOTATION_RECEIVED en ESPERA_COTIZACIONES");
                return Ok(current);
            }
            (
                ExpeditionStatus::EsperaCotizaciones,
                EventType::ComparativeMatrixConsolidated,
            ) => {
                ctx.in_progress_since = None;
                ExpeditionStatus::CuadroComparativoConsolidado
            }
            (
                ExpeditionStatus::EsperaCotizaciones,
                EventType::QuotationDeadlineExpired,
            ) => {
                ctx.in_progress_since = None;
                ExpeditionStatus::CotizacionesVencidas
            }

            // Adjudicación
            (
                ExpeditionStatus::CuadroComparativoConsolidado,
                EventType::AwardRecommendationGenerated,
            ) => ExpeditionStatus::AdjudicacionSugerida,
            (
                ExpeditionStatus::AdjudicacionSugerida,
                EventType::AwardConfirmed,
            ) => ExpeditionStatus::EnviadoRecursosFinancieros,

            // Cierre Institucional
            (
                ExpeditionStatus::EnviadoRecursosFinancieros,
                EventType::AutorizacionSubdireccionReceived,
            ) => ExpeditionStatus::AutorizadoSubdireccion,

            // Commits Finales
            (ExpeditionStatus::AutorizadoSubdireccion, EventType::UserCommit) => {
                ExpeditionStatus::Completed
            }
            (
                ExpeditionStatus::CotizacionesVencidas,
                EventType::UserCommitPartial,
            ) => ExpeditionStatus::Completed,
            // Transición de escape heredada
            (
                ExpeditionStatus::ProcedenciaAprobada,
                EventType::UserCommit,
            ) => ExpeditionStatus::Completed,

            // Transición ilegal: cualquier otro par (estado, evento)
            _ => {
                return Err(FSMTransitionError::IllegalTransition { from: current, event });
            }
        };

        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_scan_to_complete() {
        let mut ctx = FSMContext::new();
        let s0 = ExpeditionStatus::Initiated;
        let s1 = FSMEngine::transition(s0, EventType::ScanStarted, &mut ctx).unwrap();
        assert_eq!(s1, ExpeditionStatus::Scanning);
        let s2 = FSMEngine::transition(s1, EventType::DriveUploaded, &mut ctx).unwrap();
        assert_eq!(s2, ExpeditionStatus::DocsCaptured);
    }

    #[test]
    fn quotation_received_is_log_only() {
        let mut ctx = FSMContext::new();
        let s = ExpeditionStatus::EsperaCotizaciones;
        let next = FSMEngine::transition(s, EventType::QuotationReceived, &mut ctx).unwrap();
        assert_eq!(next, s); // sin cambio
    }

    #[test]
    fn terminal_state_rejects_all() {
        let mut ctx = FSMContext::new();
        let s = ExpeditionStatus::Completed;
        let res = FSMEngine::transition(s, EventType::ScanStarted, &mut ctx);
        assert!(matches!(res, Err(FSMTransitionError::TerminalState(_))));
    }

    #[test]
    fn deprecated_event_rejected() {
        let mut ctx = FSMContext::new();
        let s = ExpeditionStatus::EsperaCotizaciones;
        let res = FSMEngine::transition(s, EventType::AllQuotationsReceived, &mut ctx);
        assert!(matches!(res, Err(FSMTransitionError::DeprecatedEvent(_))));
    }

    #[test]
    fn global_validation_failed_from_any_active() {
        let mut ctx = FSMContext::new();
        for status in [
            ExpeditionStatus::Scanning,
            ExpeditionStatus::DocsCaptured,
            ExpeditionStatus::EsperaCotizaciones,
        ] {
            let next = FSMEngine::transition(status, EventType::ValidationFailed, &mut ctx).unwrap();
            assert_eq!(next, ExpeditionStatus::RejectedValidationFailed);
        }
    }

    #[test]
    fn escape_timeout_resets_to_previous() {
        let mut ctx = FSMContext::new();
        ctx.enter_in_progress();
        // Simular que ya pasó mucho tiempo (usar un timeout de 0 para forzar)
        let escaped = FSMEngine::check_escape(
            ExpeditionStatus::EsperaCotizaciones,
            &mut ctx,
            Duration::from_secs(0),
        );
        assert_eq!(escaped, Some(ExpeditionStatus::AsignacionProveedores));
    }
}