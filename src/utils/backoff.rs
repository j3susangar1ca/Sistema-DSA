//! src/utils/backoff.rs
//! Módulo: [UTIL-001] Función Estocástica de Espera (Backoff Exponencial + Jitter)
//! Cumplimiento: ISO/IEC 25010 (Reliability, Performance Efficiency)
//!
//! Implementación exacta del modelo matemático del protocolo científico v2.0:
//!   delay = min(T_base · 2^n, T_max)
//!   J ~ Uniform(0, 500ms)
//!   T_wait = delay + J
//!
//! El jitter rompe la sincronía de fase (thundering herd) cuando la red
//! del hospital se recupera tras una caída.

use rand::distributions::Uniform;
use rand::Rng;
use std::time::Duration;
use tracing::debug;

/// Generador de retardos con backoff exponencial acotado y jitter estocástico.
#[derive(Debug, Clone)]
pub struct BackoffEngine {
    base: Duration,
    max: Duration,
    max_retries: u32,
}

impl BackoffEngine {
    /// Construye el motor con parámetros validados desde `EdgeConfig`.
    pub fn new(base: Duration, max: Duration, max_retries: u32) -> Self {
        Self {
            base,
            max,
            max_retries,
        }
    }

    /// Calcula el retardo para el intento `n` (0-indexado).
    ///
    /// Fórmula:
    ///   delay = min(base · 2^n, max)
    ///   jitter ~ U(0, 500) ms
    ///   total = delay + jitter
    ///
    /// Complejidad temporal: O(1). Complejidad espacial: O(1).
    pub fn compute(&self, n: u32) -> Duration {
        let clamped_n = n.min(self.max_retries);
        let exponential = self.base * 2_u32.pow(clamped_n);
        let delay = if exponential > self.max {
            self.max
        } else {
            exponential
        };

        // Variable Aleatoria Uniforme continua en [0, 500] milisegundos.
        let mut rng = rand::thread_rng();
        let uniform = Uniform::new_inclusive(0_u64, 500_u64);
        let jitter_ms = rng.sample(uniform);

        let total = delay + Duration::from_millis(jitter_ms);

        debug!(
            target: "backoff",
            "Backoff calculado: intento={}, delay_ms={}, jitter_ms={}, total_ms={}",
            clamped_n,
            delay.as_millis(),
            jitter_ms,
            total.as_millis()
        );

        total
    }

    /// Verifica si se ha excedido el límite de reintentos permitidos.
    pub fn is_exhausted(&self, n: u32) -> bool {
        n >= self.max_retries
    }
}

/// Genera identificadores únicos canónicos para comandos y registros WAL.
pub mod ids {
    use chrono::Utc;

    /// Prefijo canónico: `cmd_dsa_<timestamp_millis>_<random_sufijo>`
    pub fn command_id() -> String {
        format!("cmd_dsa_{}_{:04x}", Utc::now().timestamp_millis(), rand::random::<u16>())
    }

    /// Prefijo para registros de buffer local.
    pub fn wal_id() -> String {
        format!("wal_{}", Utc::now().timestamp_millis())
    }

    /// UUID v4 simplificado (uso interno de trazabilidad).
    pub fn trace_id() -> String {
        format!("trace-{:08x}-{:08x}", rand::random::<u32>(), rand::random::<u32>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_monotonic_until_max() {
        let engine = BackoffEngine::new(Duration::from_secs(2), Duration::from_secs(300), 10);
        let d0 = engine.compute(0);
        let d1 = engine.compute(1);
        let d2 = engine.compute(2);
        assert!(d1 >= d0);
        assert!(d2 >= d1);
    }

    #[test]
    fn backoff_caps_at_max() {
        let engine = BackoffEngine::new(Duration::from_secs(2), Duration::from_secs(10), 10);
        let d_high = engine.compute(100); // n excede max_retries
        assert!(d_high <= Duration::from_secs(10) + Duration::from_millis(500));
    }

    #[test]
    fn jitter_within_bounds() {
        let engine = BackoffEngine::new(Duration::from_secs(0), Duration::from_secs(300), 10);
        for _ in 0..100 {
            let d = engine.compute(0);
            assert!(d <= Duration::from_millis(500));
        }
    }
}