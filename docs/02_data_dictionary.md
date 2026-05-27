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

## 2.1 Especificación del Payload del Mensaje (`Command Message`)

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

| Campo              | Tipo       | Descripción                                                                                             |
| :----------------- | :--------- | :------------------------------------------------------------------------------------------------------ |
| `command_id`       | `STRING`   | Identificador único del comando (prefijo + folio + timestamp).                                          |
| `action`           | `STRING`   | Tipo de operación: `SCRAPE_INTRANET_STATUS`, `EXCEL_UPDATE_ROW`, `EXCEL_APPEND_ROW`, `LOCAL_FILE_SYNC`. |
| `timestamp`        | `ISO8601`  | Marca temporal de emisión del comando.                                                                  |
| `requested_by`     | `STRING`   | Email del operador autenticado que detonó la acción.                                                    |
| `payload`          | `JSON`     | Parámetros específicos de la acción.                                                                    |
| `execution_status` | `STRING`   | Estado: `PENDING` → `IN_PROGRESS` → `COMPLETED` / `FAILED`.                                             |
| `response_payload` | `JSON?`    | Resultado de la ejecución (poblado por el Edge Agent).                                                  |
| `completed_at`     | `ISO8601?` | Timestamp de finalización.                                                                              |

## 2.2 Algoritmo del Consumidor en Rust (`Tokio Polling Loop`)

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

---

# 3. MÓDULOS DE DATOS Y CAPTURA
## 3.1 [LEDGER-001] MÓDULO: FONDO_REVOLVENTE_LEDGER

**ESTADO:** UNCHANGED

**Propósito:** Definir el modelo de datos canónico que unifica la representación del expediente de compra por fondo revolvente a través de todas las capas del sistema (Rust Edge Agent → SQLite WAL → BigQuery Cloud → Excel Transactive Store). Este módulo no contiene lógica de negocio propia; es la **declaración formal del schema** consumido por todos los demás módulos.

### 3.1.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-LEDGER-01] CanonicalSchemaDeclaration:**
  - **Desc:** Declarar la estructura `FondoRevolventeLedger` como el schema canónico del sistema, compuesto por 5 bloques de datos correspondientes a los hitos operativos del ciclo de vida del fondo revolvente.
  - **Logic:** Definición en Rust (fuente de verdad):

    ```rust
    use chrono::NaiveDate;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub enum EstatusTramite {
        Cotizacion,
        RecursosFinancieros,
        AutorizadoCaa,
        AutorizadoSub,
        Cancelado,
        Entregado,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct FinancieroSnapshot {
        pub precio_unitario: f64,
        pub monto_subtotal: f64,
        pub monto_iva: f64,
        pub monto_total_con_iva: f64,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct FondoRevolventeLedger {
        // Bloque 1: Ingesta e Identificación
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

        // Bloque 2: Control y Operación Interna
        pub usuario_asignado: String,
        pub fecha_inicio_cotizacion: Option<NaiveDate>,
        pub estatus_tramite: EstatusTramite,
        pub observaciones: Option<String>,

        // Bloque 3: Validación Presupuestal e Institucional (Hito SUPRE + CAA)
        pub folio_supre: Option<String>,
        pub fecha_supre: Option<NaiveDate>,
        pub paquete_envio_caa: Option<i64>,
        pub fecha_recibido_caa: Option<NaiveDate>,
        pub fecha_autorizacion_caa: Option<NaiveDate>,
        pub folio_autorizacion_caa: Option<String>,

        // Bloque 4: Adjudicación e Importes Financieros (Hito Pedido)
        pub financieros: Option<FinancieroSnapshot>,
        pub cantidad_pedido: Option<f64>,
        pub numero_pedido: Option<String>,
        pub fecha_pedido: Option<NaiveDate>,
        pub proveedor_rfc: Option<String>,

        // Bloque 5: Logística y Cierre Fiscal (Hito Pasivo/Pago)
        pub estatus_entrega: Option<String>,
        pub fecha_entrega_almacen: Option<NaiveDate>,
        pub numero_factura: Option<String>,
        pub fecha_factura: Option<NaiveDate>,
        pub fecha_envio_xml_rf: Option<NaiveDate>,
        pub fecha_pago: Option<NaiveDate>,
        pub fecha_complemento_pago_rf: Option<NaiveDate>,
    }
    ```

  - **Post-Condition:** Struct serializable a JSON/SQLite/BigQuery; todos los módulos downstream consumen esta definición como schema de referencia.

- **[ID-REQ-LEDGER-02] MilestonePopulabilityMapping:**
  - **Desc:** Documentar qué bloques del ledger se poblan en qué fase del ciclo de vida FSM, garantizando que los campos `Option<T>` se rellenan progresivamente y que ningún bloque se popula fuera de orden.
  - **Logic:** Tabla de mapping Bloque → FSM Phase:

    | Bloque         | Campos                                          | FSM Phase de Población                                      | Módulo Responsable  |
    | -------------- | ----------------------------------------------- | ----------------------------------------------------------- | ------------------- |
    | 1. Ingesta     | `folio_dsa` → `partida_especifica`              | `INITIATED` → `DOCS_CAPTURED`                               | SCAN-001 + AI-001   |
    | 2. Control     | `usuario_asignado` → `observaciones`            | `INITIATED` → `COMPLETED` (siempre activo)                  | AUTH-001 + EXP-001  |
    | 3. SUPRE + CAA | `folio_supre` → `folio_autorizacion_caa`        | `PENDING_PROCUREMENT_VERIFICATION` → `PROCEDENCIA_APROBADA` | EXP-001 + MAIL-001  |
    | 4. Pedido      | `financieros` → `proveedor_rfc`                 | `ADJUDICACION_SUGERIDA` → `COMPLETED`                       | COMP-001 + QUOT-001 |
    | 5. Pasivo/Pago | `estatus_entrega` → `fecha_complemento_pago_rf` | `COMPLETED` (post-cierre)                                   | SYNC-001            |

  - **Post-Condition:** Documento de referencia vivo que guía la implementación de cada módulo.

- **[ID-REQ-LEDGER-03] EstatusTramiteFSMBridge:**
  - **Desc:** Definir el mapeo bidireccional determinista entre los 6 valores de `EstatusTramite` (Rust/legacy) y los 20 valores de `ExpeditionStatusEnum` (FSM cloud), permitiendo traducción en ambas direcciones sin pérdida de semántica.
  - **Logic:** Tabla de mapeo:

    | EstatusTramite (Rust) | → ExpeditionStatusEnum (FSM)                                                                                      | Dirección                                                         |
    | --------------------- | ----------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
    | `Cotizacion`          | `ESPERA_COTIZACIONES`, `ASIGNACION_PROVEEDORES`, `CUADRO_COMPARATIVO_CONSOLIDADO`, `ADJUDICACION_SUGERIDA`        | FSM → Rust: cualquiera de estos 4 estados se mapea a `Cotizacion` |
    | `RecursosFinancieros` | `ENVIADO_RECURSOS_FINANCIEROS`                                                                                    | FSM → Rust                                                        |
    | `AutorizadoCaa`       | `PROCEDENCIA_APROBADA`                                                                                            | FSM → Rust                                                        |
    | `AutorizadoSub`       | `AUTORIZADO_SUBDIRECCION`                                                                                         | FSM → Rust                                                        |
    | `Cancelado`           | `REJECTED_VALIDATION_FAILED`, `REJECTED_CATALOG_INACTIVE`, `REJECTED_PROCUREMENT_DENIED`, `COTIZACIONES_VENCIDAS` | FSM → Rust: cualquiera de estos se mapea a `Cancelado`            |
    | `Entregado`           | `COMPLETED`                                                                                                       | FSM → Rust                                                        |

  - **Post-Condition:** Función de traducción `fn fsm_to_rust(status: ExpeditionStatusEnum) -> EstatusTramite` implementable sin ambigüedad.

- **[ID-REQ-LEDGER-04] TypeSafetyEnforcement:**
  - **Desc:** Garantizar que todos los campos numéricos financieros usen `f64` en Rust y `NUMERIC`/`DECIMAL(18,4)` en SQL, que las fechas sean `NaiveDate` (sin timezone implícita) y que los campos opcionales usen `Option<T>` en lugar de valores centinela (`""`, `0`, `"N/A"`).
  - **Logic:** Validación en el pipeline de transformación: `csv_async` parse → `Result<T, ParseError>` → `None` para campos faltantes, nunca string vacío.
  - **Post-Condition:** Cero valores centinela en la base de datos.

### 3.1.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `FondoRevolventeLedger` _(schema canónico — BigQuery DDL)_

  ```sql
  CREATE TABLE `proyecto.hospital_civil.fondo_revolvente_ledger` (
    -- Bloque 1: Ingesta e Identificación
    folio_dsa                 STRING      NOT NULL,
    tipo_tramite              STRING      NOT NULL DEFAULT 'COMPRA POR FONDO',
    fecha_recepcion           DATE        NOT NULL,
    servicio_solicitante      STRING      NOT NULL,
    oficio_solicitud          STRING      NOT NULL,
    codigo                    STRING(10)  NOT NULL,
    descripcion               STRING      NOT NULL,
    cantidad_solicitada       NUMERIC     NOT NULL,
    unidad_medida             STRING      NOT NULL,
    partida_especifica        STRING(4)   NOT NULL,

    -- Bloque 2: Control y Operación Interna
    usuario_asignado          STRING      NOT NULL,
    fecha_inicio_cotizacion   DATE,
    estatus_tramite           STRING      NOT NULL,
    observaciones             STRING,

    -- Bloque 3: Hito SUPRE + CAA
    folio_supre               STRING,
    fecha_supre               DATE,
    paquete_envio_caa         INT64,
    fecha_recibido_caa        DATE,
    fecha_autorizacion_caa    DATE,
    folio_autorizacion_caa    STRING,

    -- Bloque 4: Hito Pedido + Financieros
    precio_unitario           NUMERIC,
    monto_subtotal            NUMERIC,
    monto_iva                 NUMERIC,
    monto_total_con_iva       NUMERIC,
    cantidad_pedido           NUMERIC,
    numero_pedido             STRING,
    fecha_pedido              DATE,
    proveedor_rfc             STRING,

    -- Bloque 5: Hito Pasivo/Pago
    estatus_entrega           STRING,
    fecha_entrega_almacen     DATE,
    numero_factura            STRING,
    fecha_factura             DATE,
    fecha_envio_xml_rf        DATE,
    fecha_pago                DATE,
    fecha_complemento_pago_rf DATE,

    -- Metadatos de auditoría
    created_at                TIMESTAMP   NOT NULL,
    updated_at                TIMESTAMP   NOT NULL,

    PRIMARY KEY (folio_dsa, codigo)
  );
  ```

  - **Constraints:** PK compuesta (`folio_dsa`, `codigo`), NOT NULL en Bloques 1-2, NULLABLE en Bloques 3-5 (poblados progresivamente).

- **SQLite DDL (Edge Agent):**

  ```sql
  CREATE TABLE fondo_revolvente_ledger (
    folio_dsa                 TEXT    NOT NULL,
    tipo_tramite              TEXT    NOT NULL DEFAULT 'COMPRA POR FONDO',
    fecha_recepcion           TEXT    NOT NULL,
    servicio_solicitante      TEXT    NOT NULL,
    oficio_solicitud          TEXT    NOT NULL,
    codigo                    TEXT    NOT NULL,
    descripcion               TEXT    NOT NULL,
    cantidad_solicitada       REAL    NOT NULL,
    unidad_medida             TEXT    NOT NULL,
    partida_especifica        TEXT    NOT NULL,
    usuario_asignado          TEXT    NOT NULL,
    fecha_inicio_cotizacion   TEXT,
    estatus_tramite           TEXT    NOT NULL,
    observaciones             TEXT,
    folio_supre               TEXT,
    fecha_supre               TEXT,
    paquete_envio_caa         INTEGER,
    fecha_recibido_caa        TEXT,
    fecha_autorizacion_caa    TEXT,
    folio_autorizacion_caa    TEXT,
    precio_unitario           REAL,
    monto_subtotal            REAL,
    monto_iva                 REAL,
    monto_total_con_iva       REAL,
    cantidad_pedido           REAL,
    numero_pedido             TEXT,
    fecha_pedido              TEXT,
    proveedor_rfc             TEXT,
    estatus_entrega           TEXT,
    fecha_entrega_almacen     TEXT,
    numero_factura            TEXT,
    fecha_factura             TEXT,
    fecha_envio_xml_rf        TEXT,
    fecha_pago                TEXT,
    fecha_complemento_pago_rf TEXT,
    created_at                TEXT    NOT NULL,
    updated_at                TEXT    NOT NULL,
    sync_status               TEXT    DEFAULT 'PENDING',
    PRIMARY KEY (folio_dsa, codigo)
  );

  CREATE INDEX idx_ledger_estatus ON fondo_revolvente_ledger(estatus_tramite);
  CREATE INDEX idx_ledger_sync    ON fondo_revolvente_ledger(sync_status);
  CREATE INDEX idx_ledger_codigo  ON fondo_revolvente_ledger(codigo);
  ```

### 3.1.3 CONTRACTS & INTERFACES

- **COMPONENT:** `LedgerSerializer` | **TRIGGER:** Any write operation to SQLite or BigQuery
  - **DATA_CONTRACT (Input):** `FondoRevolventeLedger` struct (Rust)
  - **DATA_CONTRACT (Output):** Flat key-value map compatible with BigQuery `insertAll` or SQLite `INSERT`
  - **INVARIANT:** `FinancieroSnapshot` → se despliega en 4 columnas planas (`precio_unitario`, `monto_subtotal`, `monto_iva`, `monto_total_con_iva`). Si `financieros == None`, las 4 columnas se insertan como `NULL`.

- **COMPONENT:** `EstatusBridge` | **TRIGGER:** Sync operation (Rust → Excel Transactive Store)
  - **DATA_CONTRACT (Input):** `{fsm_status: ExpeditionStatusEnum}`
  - **DATA_CONTRACT (Output):** `{rust_status: EstatusTramite}`
  - **Logic:** Mapeo según tabla de [ID-REQ-LEDGER-03].

---

## 3.2 [SCAN-001] MÓDULO: DOCUMENT_CAPTURE_PIPELINE

**ESTADO:** UNCHANGED

### 3.2.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-SCAN-01] ScannerHardwareBridge:**
  - **Desc:** Iniciar secuencia de escaneo desde el navegador web hacia el HP ScanJet 3000 s mediante Scanner.js (Asprise) sobre loopback local, sin intervención de software intermedio por parte del usuario.
  - **Logic:** Frontend establece conexión WebSocket/Loopback → envía comando `SCAN_START` → driver TWAIN/WIA captura imagen → stream de bytes (PDF o Base64) retorna al Frontend.
  - **Post-Condition:** Blob binario disponible en memoria del cliente; emite evento `SCAN_COMPLETED`.

- **[ID-REQ-SCAN-02] DualDocumentIngestion:**
  - **Desc:** Capturar obligatoriamente dos documentos por expediente: (1) Oficio de Solicitud y (2) Negativa de Existencia, como precondición para activar el pipeline de inferencia.
  - **Logic:** `IF document_count < 2 THEN status = SCANNING; ELSE status = DOCS_CAPTURED AND enable_ai_pipeline`.
  - **Post-Condition:** Ambos blobs binarios almacenados en Google Drive en la carpeta del expediente; emite evento `DRIVE_UPLOADED`.

- **[ID-REQ-SCAN-03] DriveUploadWithIndexing:**
  - **Desc:** Cargar cada documento escaneado a Google Drive en una carpeta indexada por `folio_code` del expediente, generando un `drive_file_id` persistente.
  - **Logic:** `DriveApp.getFolderById(PARENT_FOLDER).createFolder(folio_code).createFile(blob)`. Retorna `drive_file_id` para almacenamiento en entidad `Document`.
  - **Post-Condition:** Archivo accesible vía API; `Document.drive_file_id` poblado.

### 3.2.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `Document`
  - **Properties:** `{id: UUID, expedition_id: UUID, document_type: DocumentTypeEnum, file_name: String, drive_file_id: String, mime_type: String, blob_size_bytes: Int64, created_at: ISO8601}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`drive_file_id`, `document_type`, `expedition_id`)

- **ENUM:** `DocumentTypeEnum` = `[OFFICIO_SOLICITUD, NEGATIVA_EXISTENCIA]`

### 3.2.3 CONTRACTS & INTERFACES

- **COMPONENT:** `ScannerBridge` | **TRIGGER:** User action → WebSocket command `SCAN_START`
  - **DATA_CONTRACT (Input):** `{scanner_id: String, resolution_dpi: Int32, color_mode: Enum[COLOR, GRAYSCALE, BW], output_format: Enum[PDF, JPEG, PNG]}`
  - **DATA_CONTRACT (Output):** `{status: Enum[SUCCESS, DEVICE_BUSY, ERROR], blob: Base64, page_count: Int32, error_message: String?}`

- **COMPONENT:** `DriveUploader` | **TRIGGER:** Event `SCAN_COMPLETED`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, document_type: DocumentTypeEnum, blob: Base64, mime_type: String}`
  - **DATA_CONTRACT (Output):** `{drive_file_id: String, web_view_link: String}`

---

## 3.3 [AI-001] MÓDULO: MULTIMODAL_INFERENCE_ENGINE

**ESTADO:** UNCHANGED

### 3.3.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-AI-01] UnifiedContextInference / ResponseSchemaV2:**
  - **Desc:** Enviar ambos documentos escaneados simultáneamente a Gemini 1.5 Flash como un solo _Contexto Unificado_ para extracción de entidades clave y metadatos, usando una estructura anidada con dos dominios semánticos separados: `datos_solicitud` y `auditoria_cumplimiento` en `snake_case`.
  - **Logic:** Request multipart con ambos PDFs + prompt de instrucciones. responseSchema:
    ```json
    {
      "type": "object",
      "properties": {
        "datos_solicitud": {
          "type": "object",
          "properties": {
            "folio_dsa": { "type": "string" },
            "codigo_insumo": { "type": "string" },
            "descripcion": { "type": "string" },
            "unidad_medida": { "type": "string" }
          },
          "required": ["folio_dsa", "codigo_insumo", "descripcion"]
        },
        "auditoria_cumplimiento": {
          "type": "object",
          "properties": {
            "coincidencia_bienes_servicios": { "type": "boolean" },
            "coincidencia_cronologica_fechas": { "type": "boolean" },
            "analisis_correlacion": { "type": "string" }
          },
          "required": [
            "coincidencia_bienes_servicios",
            "coincidencia_cronologica_fechas",
            "analisis_correlacion"
          ]
        }
      },
      "required": ["datos_solicitud", "auditoria_cumplimiento"]
    }
    ```
  - **Post-Condition:** JSON almacenado en `ValidationResult.gemini_raw_response`. Los campos `items_match` y `dates_consistent` se derivan de la respuesta.

- **[ID-REQ-AI-02] CrossDocumentItemValidation:**
  - **Desc:** Validar determinísticamente que los bienes/servicios coincidan semánticamente, mapeando el resultado desde `auditoria_cumplimiento.coincidencia_bienes_servicios`.
  - **Logic:** Si es `false`, la FSM transiciona a `REJECTED_VALIDATION_FAILED`.
  - **Post-Condition:** Bandera booleana persistida en `ValidationResult.items_match`.

- **[ID-REQ-AI-03] TemporalConsistencyCheck:**
  - **Desc:** Verificar coherencia cronológica de fechas de emisión, mapeando el resultado desde `auditoria_cumplimiento.coincidencia_cronologica_fechas`.
  - **Post-Condition:** Bandera persistida en `ValidationResult.dates_consistent`.

### 3.3.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `ValidationResult`
  - **Properties:** `{id: UUID, expedition_id: UUID, items_match: Boolean, dates_consistent: Boolean, temporal_delta_days: Int32, gemini_raw_response: JSON, discrepancies: Array<Discrepancy>, validated_at: ISO8601, correlation_analysis: String, extracted_folio_dsa: String, extracted_item_code: String, extracted_description: String, extracted_unit_of_measure: String?}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), UNIQUE(`expedition_id`). `correlation_analysis` es NOT NULL.
  - **Propiedades derivadas actualizadas:**
    - `items_match` ← `auditoria_cumplimiento.coincidencia_bienes_servicios`
    - `dates_consistent` ← `auditoria_cumplimiento.coincidencia_cronologica_fechas`
    - `correlation_analysis` ← `auditoria_cumplimiento.analisis_correlacion`
    - `extracted_folio_dsa` ← `datos_solicitud.folio_dsa`
    - `extracted_item_code` ← `datos_solicitud.codigo_insumo`
    - `extracted_description` ← `datos_solicitud.descripcion`
    - `extracted_unit_of_measure` ← `datos_solicitud.unidad_medida` (nullable)

- **EMBEDDED_TYPE:** `Discrepancy`
  - **Properties:** `{field: String, doc1_value: String, doc2_value: String, severity: Enum[WARNING, BLOCKING]}`

### 3.3.3 CONTRACTS & INTERFACES

- **COMPONENT:** `GeminiInferenceClient` | **TRIGGER:** Event `DRIVE_UPLOADED`
  - **DATA_CONTRACT (Input):**
    ```json
    {
      "contents": [
        {
          "role": "user",
          "parts": [
            {
              "inlineData": {
                "mimeType": "application/pdf",
                "data": "<BASE64_OFICIO>"
              }
            },
            {
              "inlineData": {
                "mimeType": "application/pdf",
                "data": "<BASE64_NEGATIVA>"
              }
            },
            { "text": "<SYSTEM_PROMPT_WITH_VALIDATION_RULES>" }
          ]
        }
      ],
      "generationConfig": {
        "responseMimeType": "application/json",
        "responseSchema": {
          "type": "object",
          "properties": {
            "datos_solicitud": {
              "type": "object",
              "properties": {
                "folio_dsa": { "type": "string" },
                "codigo_insumo": { "type": "string" },
                "descripcion": { "type": "string" },
                "unidad_medida": { "type": "string" }
              },
              "required": ["folio_dsa", "codigo_insumo", "descripcion"]
            },
            "auditoria_cumplimiento": {
              "type": "object",
              "properties": {
                "coincidencia_bienes_servicios": { "type": "boolean" },
                "coincidencia_cronologica_fechas": { "type": "boolean" },
                "analisis_correlacion": { "type": "string" }
              },
              "required": [
                "coincidencia_bienes_servicios",
                "coincidencia_cronologica_fechas",
                "analisis_correlacion"
              ]
            }
          },
          "required": ["datos_solicitud", "auditoria_cumplimiento"]
        }
      }
    }
    ```
  - **DATA_CONTRACT (Output):** JSON conforme al `responseSchema` anterior.

---

## 3.4 [SYNC-001] MÓDULO: EDGE_SYNCHRONIZATION_BRIDGE

**ESTADO:** PATCH_REVISION — Excel local muta de Data Mart de solo lectura a **Transactive Store** bidireccional de lectura/escritura actualizado por clave compuesta `(folio_dsa, codigo)`.

### 3.4.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-SYNC-01·P3] BidirectionalTransactiveStoreEnforcement:**
  - **Desc:** El archivo Excel local (.xlsx sobre SMB) opera como un **Transactive Store** modificable e interactivo de lectura y escritura. El daemon Rust actualiza el estatus de los trámites y el progreso de los hitos operativos a medida que ocurren las transiciones en la FSM cloud, manteniendo sincronizado el Edge de forma resiliente.
  - **Logic:** El daemon Rust realiza de forma segura el lock-check (`~$filename.xlsx`). Al detectar actualizaciones en SQLite, busca la fila correspondiente por `(folio_dsa, codigo)`. Si existe, sobrescribe los campos específicos correspondientes al hito de la FSM (Bloques 3, 4 y 5); si no existe, ejecuta un _append_ de la fila completa denormalizada.
  - **Post-Condition:** Excel local modificado y actualizado dinámicamente con cero corrupción en datos.

- **[ID-REQ-SYNC-01·P] SQLiteWALPersistence:**
  - **Desc:** El daemon Rust persiste en la base de datos SQLite local (`.db`) como Write-Ahead Log intermedio basado en la entidad canónica `FondoRevolventeLedger` antes de subir a BigQuery.
  - **Logic:** Si BigQuery es inaccesible, los registros permanecen en SQLite con `sync_status = PENDING` y se reintentan en el siguiente ciclo.
  - **Post-Condition:** Datos locales persistidos en SQLite en tránsito.

- **[ID-REQ-SYNC-01·P2] BigQueryBatchLoad:**
  - **Desc:** Reemplazar la escritura directa a Sheets por un job de carga batch (`WRITE_APPEND`) a BigQuery desde el agente Rust, consumiendo registros de SQLite.
  - **Post-Condition:** Filas disponibles en BigQuery; registros locales marcados como `SYNCED`.

- **[ID-REQ-SYNC-02] PessimisticFileLockDetection:**
  - **Desc:** Verificar ausencia del archivo de bloqueo temporal (`~$<NombreDelArchivo>.xlsx`) antes de realizar cualquier actualización o append en el Excel local. Si existe, aplicar Exponential Backoff y suspender la escritura.
  - **Post-Condition:** Fila insertada o actualizada en Excel local seguro sin corrupción.

- **[ID-REQ-SYNC-03] ExponentialBackoffRetry:**
  - **Desc:** Reintento exponencial con techo de 300 segundos para la I/O de Excel y persistencia del puntero para evitar re-procesamiento.
  - **Post-Condition:** Estado `SYNC_BLOCKED` si se exceden 10 reintentos.

- **[ID-REQ-SYNC-04] DriveToSMBFileSync:**
  - **Desc:** El daemon Rust replica archivos nuevos depositados en Google Drive local al directorio SMB correspondiente.
  - **Logic:** File watcher en Drive local → copia a `SMB_EXPEDIENTES/<folio_code>/`.
  - **Post-Condition:** Archivo disponible en ambas ubicaciones.

- **[ID-REQ-SYNC-05] LegacyCSVPersistence:**
  - **Desc:** Los CSV de xfarma se parsean en chunks y se transforman en registros `FondoRevolventeLedger` para ser insertados en SQLite antes de BigQuery.
  - **Post-Condition:** Datos legacy cargados transaccionalmente de manera atómica.

- **[ID-REQ-SYNC-06] TransactiveRowUpdateOrAppend:**
  - **Desc:** Escribir y actualizar filas en el Excel local según el `folio_dsa` y `codigo` del insumo, mapeando dinámicamente los campos actualizados de `FondoRevolventeLedger` (Bloques 3, 4 y 5) conforme progresan las fases operativas cloud, o realizando un full row append si no existe.
  - **Logic:** Mapea el struct completo denormalizado al layout de columnas en Excel. Busca por clave primaria compuesta `(folio_dsa, codigo)`. Ejecuta actualización in-situ si coincide, previniendo incoherencias transaccionales.

#### 3.4.1.1 — Política de Control de Concurrencia Pesimista e Integridad de Red (Rust-to-SMB)

El archivo Microsoft Excel ubicado en la red local SMB opera como un **Transactive Store** bidireccional. Para evitar condiciones de carrera (_Data Race_) o corrupción por bloqueos concurrentes de usuarios humanos, el Agente en Rust implementa un subsistema de aislamiento mediante exclusión mutua basada en el sistema de archivos de Windows 11.

##### 3.4.1.1.1 Mecanismo Antibloqueo (_Starvation Prevention Protocol_)

**Paso 1 — Detección Atómica del Lock:**
Antes de iniciar cualquier escritura, el hilo de Rust busca la existencia del archivo descriptor de bloqueo oculto generado nativamente por Microsoft Excel:

```
IF EXISTS(~$<NombreDelArchivo>.xlsx)
  THEN lock_detected = true
  ELSE lock_detected = false
```

**Paso 2 — Estrategia ante Lock Activo (Usuario editando localmente):**

- El agente **no aborta** la operación.
- El agente **no genera un archivo duplicado** ("copia en conflicto"), manteniendo la integridad de la red SMB limpia.
- El agente suspende la escritura en el Excel y almacena la transacción pendiente en el caché de persistencia local seguro de **SQLite (`local_buffer.db`)** operando en modo _Write-Ahead Log (WAL)_.

**Paso 3 — Exponential Backoff:**

```
delay = min(base_delay * 2^retry_counter, max_delay_300s)
// base_delay = 2 segundos
// max_delay = 300 segundos (5 minutos)
// max_retries = 10
// Si retry_counter > max_retries → status = SYNC_BLOCKED
//   → Notificar a UI del operador
//   → Preservar last_processed_id en Sheets/BigQuery
```

**Paso 4 — Liberación y Vaciado (_Flushing_):**
Un hilo de vigilancia con estrategia de _Exponential Backoff_ testea el descriptor de bloqueo de Windows. En el instante en que el usuario humano cierra el Excel local y el archivo `~$` desaparece:

1. El daemon de Rust toma el control exclusivo del archivo mediante la API Win32 (`FileShare.None`).
2. Extrae en bloque (_Batch Read_) los registros acumulados en la SQLite local.
3. Ejecuta una operación de tipo UPDATE o APPEND de filas utilizando el crate `calamine`/`openpyxl-rs`.
4. Libera el manejador del archivo inmediatamente tras la escritura.
5. Actualiza `sync_status` de los registros en SQLite a `SYNCED`.

##### 3.4.1.1.2 Propiedades de Consistencia

| Propiedad                          | Garantía                                                                                                                                |
| :--------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------- |
| **Consistencia eventual perfecta** | Los datos jamás se pierden: la nube (BigQuery) es la fuente de verdad; SQLite es el buffer resiliente.                                  |
| **Cero corrupción de archivos**    | Nunca se escribe en Excel mientras el lock `~$` exista.                                                                                 |
| **Cero pérdida ante caída de red** | SQLite WAL persiste localmente. Upload a BigQuery se reintenta indefinidamente.                                                         |
| **Tablas dinámicas intactas**      | Las fórmulas y tablas dinámicas nativas de la `Hoja3` del Excel se recalculan limpiamente al cerrar y reabrir, sin intervención manual. |
| **No duplicación de archivos**     | No se generan "copias en conflicto" ni archivos temporales en la red SMB.                                                               |

##### 3.4.1.1.3 Diagrama de Flujo de Decisión

```
                    ┌─────────────────┐
                    │ Registro        │
                    │ pendiente en    │
                    │ SQLite WAL      │
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │ ¿Existe lock    │
                    │ ¿~$archivo.xlsx?│
                    └────┬───────┬────┘
                         │       │
                    SÍ   │       │   NO
                         ▼       ▼
                ┌──────────┐  ┌──────────────┐
                │ Suspende │  │ Win32        │
                │ Escritura│  │ FileShare    │
                │          │  │ .None        │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
                ┌──────────┐  ┌──────────────┐
                │ Aplica   │  │ Batch Read   │
                │ Backoff  │  │ desde SQLite │
                │ (2^n seg)│  │              │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
                ┌──────────┐  ┌──────────────┐
                │ Re-test  │  │ UPDATE/      │
                │ Lock     │  │ APPEND en    │
                │          │  │ Excel        │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
               ┌──────────┐   ┌──────────────┐
               │ ¿Libre?  │   │ Libera       │
               │ SÍ → Loop│   │ Handle       │
               │ NO → Wait│   │ sync_status  │
               └──────────┘   │ = SYNCED     │
                              └──────────────┘
```

### 3.4.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `SyncPointer`
  - **NOTA:** Entidad parcialmente deprecada. La función de `last_processed_row_id` es subsumida por `FondoRevolventeLedger.sync_status` en SQLite. Se mantiene para compatibilidad con el polling de Sheets (Google Sheets como control plane).
  - **Properties:** `{id: UUID, sheet_id: String, last_processed_row_id: Int64, last_processed_expedition_id: UUID, retry_count: Int32, status: SyncStatusEnum, updated_at: ISO8601}`
  - **Constraints:** PK(`id`), UNIQUE(`sheet_id`), NOT NULL(`last_processed_row_id`)

- **ENTITY:** `SQLiteWAL`
  - **Properties:** `{record_id: UUID, table_name: String, payload: JSON, status: WALStatusEnum, created_at: ISO8601, synced_at: ISO8601?}`
  - **Constraints:** PK(`record_id`), NOT NULL(`table_name`, `payload`, `status`)

- **ENTITY:** `BigQueryLoadJob`
  - **Properties:** `{job_id: String, dataset_id: String, table_id: String, row_count: Int64, status: BQJobStatusEnum, created_at: ISO8601, completed_at: ISO8601?}`
  - **Constraints:** PK(`job_id`), NOT NULL(`status`)

- **ENUM:** `SyncStatusEnum` = `[IN_SYNC, PENDING_RETRY, SYNC_BLOCKED, UP_TO_DATE]`
- **ENUM:** `WALStatusEnum` = `[PENDING, UPLOADING, SYNCED, FAILED]`
- **ENUM:** `BQJobStatusEnum` = `[RUNNING, DONE, FAILED]`

- **Excel Column Layout (v4.1 canonical Transactive Store):**
  ```
  | A: folio_dsa | B: tipo_tramite | C: fecha_recepcion | D: servicio_solicitante |
  | E: oficio_solicitud | F: codigo | G: descripcion | H: cantidad_solicitada |
  | I: unidad_medida | J: partida_especifica | K: usuario_asignado |
  | L: fecha_inicio_cotizacion | M: estatus_tramite | N: observaciones |
  | O: folio_supre | P: fecha_supre | Q: paquete_envio_caa |
  | R: fecha_recibido_caa | S: fecha_autorizacion_caa | T: folio_autorizacion_caa |
  | U: precio_unitario | V: monto_subtotal | W: monto_iva | X: monto_total_con_iva |
  | Y: cantidad_pedido | Z: numero_pedido | AA: fecha_pedido | AB: proveedor_rfc |
  | AC: estatus_entrega | AD: fecha_entrega_almacen | AE: numero_factura |
  | AF: fecha_factura | AG: fecha_envio_xml_rf | AH: fecha_pago |
  | AI: fecha_complemento_pago_rf |
  ```

### 3.4.3 CONTRACTS & INTERFACES

- **COMPONENT:** `SheetsPoller` | **TRIGGER:** Cron/Rust tokio interval
  - **DATA_CONTRACT (Input):** `{sheet_id: String, range: String, last_processed_row_id: Int64}`
  - **DATA_CONTRACT (Output):** `{new_rows: Array<FondoRevolventeLedger>, has_more: Boolean}`

- **COMPONENT:** `ExcelWriter` | **TRIGGER:** Queue drained + lock released
  - **DATA_CONTRACT (Input):** `{file_path: String, records: Array<FondoRevolventeLedger>, timeout: Duration}`
  - **DATA_CONTRACT (Output):** `{written: Int32, lock_detected: Boolean, new_pointer: Int64}`
  - **INVARIANT:** Realiza búsquedas llave `(folio_dsa, codigo)` para ejecutar UPDATE local en celdas, o inserta mediante APPEND si no existe registro previo.

- **COMPONENT:** `FilesystemWatcher` | **TRIGGER:** filesystem notification on sync folder
  - **DATA_CONTRACT (Input):** `{watch_path: String, event_type: Enum[CREATED, MODIFIED]}`
  - **DATA_CONTRACT (Output):** `{file_id: String, file_name: String, target_smb_path: String}`

- **COMPONENT:** `SQLiteManager` | **TRIGGER:** CSV file detected OR queued writes from any module
  - **DATA_CONTRACT (Input):** `{table_name: String, rows: Array<JSON>, transaction_mode: Enum[IMMEDIATE, DEFERRED]}`
  - **DATA_CONTRACT (Output):** `{inserted: Int32, transaction_committed: Boolean}`

- **COMPONENT:** `BigQueryBatchLoader` | **TRIGGER:** Cron interval OR threshold reached
  - **DATA_CONTRACT (Input):** `{project_id: String, dataset_id: String, table_id: String, pending_records: Array<SQLiteWAL>}`
  - **DATA_CONTRACT (Output):** `{job_id: String, rows_loaded: Int64, errors: Array<String>?}`
