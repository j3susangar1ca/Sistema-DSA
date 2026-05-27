/**
 * CONSTANTS.GS
 * Módulo de Configuración Inmutable
 * ISO 25010: Maintainability
 */
const CONFIG = {
  // Identificadores de Recursos (Rellenar con IDs reales del proyecto)
  QUEUE_FILE_ID: 'REEMPLAZAR_CON_ID_ARCHIVO_QUEUE_JSON',
  MASTER_SHEET_ID: 'REEMPLAZAR_CON_ID_HOJA_CONTROL',
  BQ_PROJECT_ID: 'hospital-civil-4562',
  BQ_DATASET: 'hospital_civil',
  
  // Cache y Performance
  CACHE_TTL_WHITELIST: 21600, // 6 horas
  CACHE_TTL_QUEUE: 30,        // 30 segundos para reducir lecturas de Drive
  POLLING_THRESHOLD: 50,      // Max comandos por ciclo de polling
  
  // Auditoría
  AUDIT_SHEET_NAME: 'Audit_Log',
  EVENTS_SHEET_NAME: 'Events',
  ACCESS_SHEET_NAME: 'Control_Acceso'
};
