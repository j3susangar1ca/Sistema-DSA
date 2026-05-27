# 2. PROTOCOLO DE INTEROPERABILIDAD CLOUD-EDGE

Para cumplir con la directriz de **Cero Puertos Entrantes Abiertos (Zero Inbound Ports)** — [RNF-004] — en la red del hospital, la comunicación entre la infraestructura Serverless de Google (Cloud) y el entorno físico Windows 11 (Edge) se implementa mediante un patrón de **Cola de Comandos Inversa (Asynchronous Command Queue)**.

```
[ APPS SCRIPT WEB APP ]
          │
          ▼ (1) Registra comando transaccional en búfer indexado
[ GOOGLE DRIVE (queue.json) / BigQuery scraping_requests ]
          ▲
          │ (2) Polling de baja latencia (intervalo 1s vía Tokio Async Thread)
[ EDGE WORKER (Rust Runtime) ] ──► Ejecuta acción local ──► [ RECURSO LOCAL ]
          │                         (ej. Web Scraping Intranet,
          │                          Excel Update, CSV Parse)
          ▼ (3) Serializa payload JSON de respuesta y limpia registro de cola
[ GOOGLE DRIVE (response.json) / BigQuery scraping_requests ]
```

## 2.1.1 Especificación del Payload del Mensaje (`Command Message`)

Cuando el frontend web requiere una acción que exige privilegios de red local o acceso a hardware local, el backend de Apps Script escribe un objeto estructurado en la capa intermedia de sincronización:

```json
{
  "command_id": "cmd_249_1719468233",
  "action": "SCRAPE_INTRANET_STATUS",
  "timestamp": "2026-05-27T12:23:53Z",
  "requested_by": "operador_a@hcg.gob.mx",
  "payload": {
    "expedition_id": "UUID-del-expediente",
    "codigo_insumo": "2541004446",
    "partida_conac": "2541"
  },
  "execution_status": "PENDING",
  "response_payload": null,
  "completed_at": null
}
```

**Campos del Command Message:**

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `command_id` | `STRING` | Identificador único del comando (prefijo + folio + timestamp). |
| `action` | `STRING` | Tipo de operación: `SCRAPE_INTRANET_STATUS`, `EXCEL_UPDATE_ROW`, `EXCEL_APPEND_ROW`, `LOCAL_FILE_SYNC`. |
| `timestamp` | `ISO8601` | Marca temporal de emisión del comando. |
| `requested_by` | `STRING` | Email del operador autenticado que detonó la acción. |
| `payload` | `JSON` | Parámetros específicos de la acción. |
| `execution_status` | `STRING` | Estado: `PENDING` → `IN_PROGRESS` → `COMPLETED` / `FAILED`. |
| `response_payload` | `JSON?` | Resultado de la ejecución (poblado por el Edge Agent). |
| `completed_at` | `ISO8601?` | Timestamp de finalización. |

## 2.1.2 Algoritmo del Consumidor en Rust (`Tokio Polling Loop`)

El daemon en Rust mantiene un hilo asíncronizado sin bloqueo de CPU a través de un bucle de lectura de ultra-baja latencia:

```rust
use tokio::time::{interval, Duration};

async fn command_polling_loop() {
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        // Paso 1: Leer cola de comandos pendientes
        let pending = fetch_pending_commands().await;

        for cmd in pending {
            // Paso 2: Marcar como IN_PROGRESS para evitar doble ejecución
            mark_in_progress(&cmd.command_id).await;

            // Paso 3: Ejecutar según tipo de acción
            let result = match cmd.action.as_str() {
                "SCRAPE_INTRANET_STATUS" => {
                    execute_intranet_scrape(&cmd.payload).await
                },
                "EXCEL_UPDATE_ROW" => {
                    execute_excel_update(&cmd.payload).await
                },
                "EXCEL_APPEND_ROW" => {
                    execute_excel_append(&cmd.payload).await
                },
                "LOCAL_FILE_SYNC" => {
                    execute_file_sync(&cmd.payload).await
                },
                _ => Err(format!("Unknown action: {}", cmd.action)),
            };

            // Paso 4: Serializar resultado y limpiar cola
            match result {
                Ok(payload) => {
                    mark_completed(&cmd.command_id, &payload).await;
                },
                Err(e) => {
                    mark_failed(&cmd.command_id, &e).await;
                }
            }
        }
    }
}
```

**Propiedades del patrón:**

- **Zero Inbound Ports:** El Edge Agent únicamente realiza conexiones _outbound_ (HTTPS hacia BigQuery/Drive).
- **Idempotencia:** El campo `execution_status` previene la re-ejecución de comandos ya procesados.
- **At-least-once delivery:** Si el Edge falla durante la ejecución, el comando permanece `IN_PROGRESS` y se resetea a `PENDING` tras un timeout configurable (default 60s).
- **Observabilidad:** Cada comando deja traza en `AccessAuditLog` (actor) y en `ExpeditionEvent` (resultado).
