/**
 * DRIVE_QUEUE.GS
 * Módulo: [SYNC-001] Gestión de Cola Inversa (Command Queue)
 * Optimización: Uso de CacheService para reducir I/O de Drive en polling frecuente.
 */

/**
 * Obtiene la cola de comandos. Prioriza caché de lectura para alta frecuencia.
 * @returns {Array} - Lista de comandos pendientes.
 */
function getCommandQueue() {
  const cache = CacheService.getScriptCache();
  const cacheKey = 'dsa_command_queue';
  const cached = cache.get(cacheKey);

  if (cached) {
    return JSON.parse(cached);
  }

  // Lectura de Drive (Costosa)
  try {
    const file = DriveApp.getFileById(CONFIG.QUEUE_FILE_ID);
    const content = file.getBlob().getDataAsString();
    const queueData = content ? JSON.parse(content) : { commands: [] };
    
    // Cachear por tiempo corto para aliviar carga en polling de 1s
    cache.put(cacheKey, JSON.stringify(queueData), CONFIG.CACHE_TTL_QUEUE);
    return queueData.commands || [];

  } catch (e) {
    console.error(`Error leyendo cola de Drive: ${e.message}`);
    return [];
  }
}

/**
 * Actualiza la cola de comandos y limpia la caché.
 * @param {Array} commands - Array completo de comandos actualizado.
 */
function updateCommandQueue(commands) {
  const queueData = { commands: commands, updated_at: new Date().toISOString() };
  const content = JSON.stringify(queueData, null, 2);

  try {
    const file = DriveApp.getFileById(CONFIG.QUEUE_FILE_ID);
    // setContent es atómico y eficiente
    file.setContent(content);
    
    // Invalidar caché de lectura inmediatamente
    CacheService.getScriptCache().remove('dsa_command_queue');
    
    return true;
  } catch (e) {
    console.error(`Error escribiendo cola de Drive: ${e.message}`);
    return false;
  }
}

/**
 * Añade un nuevo comando a la cola.
 * @param {Object} command - Objeto comando estructurado.
 */
function pushCommand(command) {
  const commands = getCommandQueue();
  commands.push(command);
  return updateCommandQueue(commands);
}
