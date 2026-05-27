/**
 * AUTH_MANAGER.GS
 * Módulo: [AUTH-001] Control de Acceso Federado y Caché
 */

/**
 * Valida el acceso del usuario contra la whitelist con TTL de 6h.
 * @param {string} email - Correo del usuario autenticado.
 * @returns {boolean} - True si está autorizado.
 */
function isUserAuthorized(email) {
  if (!email) return false;

  const cache = CacheService.getScriptCache();
  const cacheKey = `whitelist_${email}`;
  const cachedStatus = cache.get(cacheKey);

  if (cachedStatus) {
    return cachedStatus === '1';
  }

  // Cache Miss: Validación pesada contra Sheet
  try {
    const sheet = SpreadsheetApp.openById(CONFIG.MASTER_SHEET_ID).getSheetByName(CONFIG.ACCESS_SHEET_NAME);
    const lastRow = sheet.getLastRow();
    
    if (lastRow < 2) return false; // Lista vacía

    // Optimización: Leer solo la columna de emails
    const emails = sheet.getRange(2, 1, lastRow - 1, 1).getValues().flat();
    const isAuthorized = emails.includes(email);

    // Persistir en cache (1 = Autorizado, 0 = Denegado)
    cache.put(cacheKey, isAuthorized ? '1' : '0', CONFIG.CACHE_TTL_WHITELIST);
    return isAuthorized;

  } catch (e) {
    console.error(`Error crítico en AuthManager: ${e.message}`);
    return false; // Fail-Secure
  }
}

/**
 * Log de auditoría no bloqueante (Fire & Forget).
 * Escribe en una cola interna de logs si la celda está disponible, 
 * pero no detiene la ejecución principal.
 */
function logAuditAsync(email, action, status, details) {
  try {
    const timestamp = new Date();
    // Uso de LockService para prevenir race conditions en escritura de logs
    const lock = LockService.getScriptLock();
    if (lock.tryLock(2000)) { // Espera máxima 2s
      const sheet = SpreadsheetApp.openById(CONFIG.MASTER_SHEET_ID).getSheetByName(CONFIG.AUDIT_SHEET_NAME);
      sheet.appendRow([timestamp, email, action, status, JSON.stringify(details)]);
      lock.releaseLock();
    }
  } catch (e) {
    console.warn(`Fallo en auditoría asíncrona: ${e.message}`);
  }
}
