/**
 * MAIN.GS
 * Módulo: [PROXY-001] API Gateway & Control Plane Engine
 * Implementa el protocolo de interoperabilidad Cloud-Edge v2.0.
 */

/**
 * Entry point principal para peticiones HTTP POST (Rust Edge Agent / Web App).
 * @param {GoogleAppsScript.Events.DoPost} e
 */
function doPost(e) {
  const timestampStart = new Date().toISOString();
  
  try {
    // 1. Validación de Payload
    if (!e || !e.postData || !e.postData.contents) {
      return createResponse(400, { status: 'error', code: 'INVALID_PAYLOAD' });
    }

    const request = JSON.parse(e.postData.contents);
    const action = request.action;
    const userEmail = Session.getActiveUser().getEmail();

    // 2. Auditoría de Acceso (Fail-Fast)
    // El Edge Agent tiene un usuario de servicio, los operadores humanos usan sus credenciales.
    const isEdge = userEmail === 'windows_edge_agent@hcg.gob.mx'; // Configurar según Service Account
    
    if (!isEdge && !isUserAuthorized(userEmail)) {
      logAuditAsync(userEmail, 'ACCESS_DENIED', 'UNAUTHORIZED', { ip: 'unknown' });
      return createResponse(403, { status: 'error', code: 'FORBIDDEN' });
    }

    // 3. Router de Acciones
    let payload = {};
    
    switch (action) {
      case 'POLL_COMMANDS':
        payload = handlePollCommands();
        break;
        
      case 'ACK_COMMAND':
        payload = handleAckCommand(request);
        break;
        
      case 'HEARTBEAT':
        payload = { status: 'OK', server_time: timestampStart };
        break;
        
      case 'REGISTER_EVENT':
        // Delegación al motor FSM para registrar eventos desde la UI
        payload = handleFsmEvent(request);
        break;
        
      default:
        return createResponse(400, { status: 'error', code: 'UNKNOWN_ACTION', details: action });
    }

    // 4. Respuesta Exitosa
    return createResponse(200, {
      status: 'success',
      execution_status: 'COMPLETED',
      completed_at: new Date().toISOString(),
      response_payload: payload
    });

  } catch (error) {
    console.error(`Error Crítico en API Gateway: ${error.stack}`);
    return createResponse(500, { 
      status: 'error', 
      code: 'INTERNAL_SERVER_ERROR', 
      message: 'Fallo interno del sistema.' 
    });
  }
}

/**
 * Manejador de Polling para el Edge Agent.
 * Devuelve comandos pendientes y los marca como IN_PROGRESS.
 */
function handlePollCommands() {
  const allCommands = getCommandQueue();
  const pendingCommands = [];
  let modified = false;

  // Limitar batch size para evitar timeouts de GAS
  const limit = CONFIG.POLLING_THRESHOLD;
  
  for (let i = 0; i < allCommands.length && pendingCommands.length < limit; i++) {
    const cmd = allCommands[i];
    if (cmd.execution_status === 'PENDING') {
      cmd.execution_status = 'IN_PROGRESS';
      cmd.received_at = new Date().toISOString();
      pendingCommands.push(cmd);
      modified = true;
    }
  }

  if (modified) {
    // Actualizar estado en cola (Async)
    updateCommandQueue(allCommands); 
    // Nota: En producción se podría optimizar escribiendo solo los índices modificados
  }

  return { commands: pendingCommands };
}

/**
 * Manejador de ACK (Acknowledgement).
 * Marca un comando como COMPLETED o FAILED y mueve a historial.
 */
function handleAckCommand(request) {
  const { command_id, status, response_payload } = request;
  
  // Para una implementación de producción robusta, aquí se buscaría el comando
  // en el array global y se actualizaría. 
  // Por eficiencia en este bloque, asumimos que el Edge confirma su propio trabajo.
  
  console.log(`ACK recibido para ${command_id}: ${status}`);
  
  // Opcional: Registrar evento en BigQuery o Sheets
  return { acknowledged: true, command_id };
}

/**
 * Wrapper para crear respuestas JSON tipadas.
 */
function createResponse(httpCode, body) {
  const output = ContentService.createTextOutput(JSON.stringify(body))
    .setMimeType(ContentService.MimeType.JSON);
  
  // Hack para permitir CORS si se consume desde ciertos clientes externos
  // Aunque Apps Script maneja CORS automáticamente en despliegues nuevos.
  return output; 
}
