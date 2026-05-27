/**
 * SISTEMA-DSA: CONTROL PLANE ENGINE
 * Módulo: [PROXY-001] / [AUTH-001] API Gateway
 * Convención de nombres: snake_case para payloads de persistencia
 */

function doPost(e) {
  var timestamp_inicio = new Date().toISOString();

  try {
    // 1. Validar integridad de la solicitud entrante
    if (!e || !e.postData || !e.postData.contents) {
      return generarRespuestaJson(
        {
          status: "error",
          error_code: "INVALID_MULTIPART_REQUEST",
          message: "Payload vacío o malformado.",
        },
        400,
      );
    }

    // 2. Parsear el Command Message entrante
    var datos_entrantes = JSON.parse(e.postData.contents);
    var accion_solicitada = datos_entrantes.action;
    var operador = datos_entrantes.requested_by || "sistema_edge@hcg.gob.mx";

    // 3. Evaluar el tipo de acción (Protocolo Cloud-Edge)
    var respuesta_payload = {};

    if (accion_solicitada === "POLL_COMMANDS") {
      // Simulación de lectura de queue.json o base analítica BigQuery
      respuesta_payload = fetch_proximo_comando_pendiente(operador);
    } else if (accion_solicitada === "ACK_COMMAND") {
      // Confirmación y cambio de estado en la FSM
      respuesta_payload = registrar_confirmacion_comando(
        datos_entrantes.command_id,
        datos_entrantes.status,
      );
    } else {
      // Inyección genérica de chequeo de conectividad (Heartbeat)
      respuesta_payload = {
        status: "HEARTBEAT_OK",
        server_timestamp: timestamp_inicio,
      };
    }

    // 4. Retornar HTTP 200 OK estructurado de manera conforme
    return generarRespuestaJson({
      status: "success",
      execution_status: "COMPLETED",
      completed_at: new Date().toISOString(),
      response_payload: respuesta_payload,
    });
  } catch (error) {
    // Log de auditoría asíncrona ante fallos críticos
    Logger.log("Error crítico en API Gateway: " + error.toString());

    return generarRespuestaJson(
      {
        status: "error",
        execution_status: "FAILED",
        error_code: "CRITICAL_SERVER_EXCEPTION",
        message: error.toString(),
      },
      500,
    );
  }
}

/**
 * Helper de encapsulamiento para forzar cabeceras de Content-Type e inmutabilidad
 */
function generarRespuestaJson(objeto_salida, codigo_http) {
  var json_string = JSON.stringify(objeto_salida);
  return ContentService.createTextOutput(json_string).setMimeType(
    ContentService.MimeType.JSON,
  );
}

/**
 * Mock de infraestructura simulando la cola transaccional sobre Drive
 */
function fetch_proximo_comando_pendiente(operador) {
  // En producción, esto consulta la base o lee queue.json en Drive
  return {
    command_id: "cmd_dsa_" + new Date().getTime(),
    action: "HEARTBEAT_TEST",
    payload: {
      mensaje: "Conexión exitosa con Execution Plane Edge.",
    },
  };
}

function registrar_confirmacion_comando(command_id, status) {
  return {
    command_id: command_id,
    synchronized: true,
    fsm_updated: true,
  };
}
