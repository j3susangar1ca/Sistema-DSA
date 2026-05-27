---

## 4. PROTOCOLO DE INTEROPERABILIDAD CLOUD-EDGE

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

### 4.1. Especificación del Payload del Mensaje (`Command Message`)

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

### 4.2. Algoritmo del Consumidor en Rust (`Tokio Polling Loop`)

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

### [LEDGER-001] MÓDULO: FONDO_REVOLVENTE_LEDGER

**ESTADO:** UNCHANGED

**Propósito:** Definir el modelo de datos canónico que unifica la representación del expediente de compra por fondo revolvente a través de todas las capas del sistema (Rust Edge Agent → SQLite WAL → BigQuery Cloud → Excel Transactive Store). Este módulo no contiene lógica de negocio propia; es la **declaración formal del schema** consumido por todos los demás módulos.

#### H4.1. REQUERIMIENTOS FUNCIONALES

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

#### H4.2. PERSISTENCIA Y DATA MODEL

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

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `LedgerSerializer` | **TRIGGER:** Any write operation to SQLite or BigQuery
  - **DATA_CONTRACT (Input):** `FondoRevolventeLedger` struct (Rust)
  - **DATA_CONTRACT (Output):** Flat key-value map compatible with BigQuery `insertAll` or SQLite `INSERT`
  - **INVARIANT:** `FinancieroSnapshot` → se despliega en 4 columnas planas (`precio_unitario`, `monto_subtotal`, `monto_iva`, `monto_total_con_iva`). Si `financieros == None`, las 4 columnas se insertan como `NULL`.

- **COMPONENT:** `EstatusBridge` | **TRIGGER:** Sync operation (Rust → Excel Transactive Store)
  - **DATA_CONTRACT (Input):** `{fsm_status: ExpeditionStatusEnum}`
  - **DATA_CONTRACT (Output):** `{rust_status: EstatusTramite}`
  - **Logic:** Mapeo según tabla de [ID-REQ-LEDGER-03].

---

### [SCAN-001] MÓDULO: DOCUMENT_CAPTURE_PIPELINE

**ESTADO:** UNCHANGED

#### H4.1. REQUERIMIENTOS FUNCIONALES

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

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `Document`
  - **Properties:** `{id: UUID, expedition_id: UUID, document_type: DocumentTypeEnum, file_name: String, drive_file_id: String, mime_type: String, blob_size_bytes: Int64, created_at: ISO8601}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`drive_file_id`, `document_type`, `expedition_id`)

- **ENUM:** `DocumentTypeEnum` = `[OFFICIO_SOLICITUD, NEGATIVA_EXISTENCIA]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `ScannerBridge` | **TRIGGER:** User action → WebSocket command `SCAN_START`
  - **DATA_CONTRACT (Input):** `{scanner_id: String, resolution_dpi: Int32, color_mode: Enum[COLOR, GRAYSCALE, BW], output_format: Enum[PDF, JPEG, PNG]}`
  - **DATA_CONTRACT (Output):** `{status: Enum[SUCCESS, DEVICE_BUSY, ERROR], blob: Base64, page_count: Int32, error_message: String?}`

- **COMPONENT:** `DriveUploader` | **TRIGGER:** Event `SCAN_COMPLETED`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, document_type: DocumentTypeEnum, blob: Base64, mime_type: String}`
  - **DATA_CONTRACT (Output):** `{drive_file_id: String, web_view_link: String}`

---

### [AI-001] MÓDULO: MULTIMODAL_INFERENCE_ENGINE

**ESTADO:** UNCHANGED

#### H4.1. REQUERIMIENTOS FUNCIONALES

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

#### H4.2. PERSISTENCIA Y DATA MODEL

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

#### H4.3. CONTRACTS & INTERFACES

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

### [SYNC-001] MÓDULO: EDGE_SYNCHRONIZATION_BRIDGE

**ESTADO:** PATCH_REVISION — Excel local muta de Data Mart de solo lectura a **Transactive Store** bidireccional de lectura/escritura actualizado por clave compuesta `(folio_dsa, codigo)`.

#### H4.1. REQUERIMIENTOS FUNCIONALES

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

#### SYNC-001 H4.1.1 — Política de Control de Concurrencia Pesimista e Integridad de Red (Rust-to-SMB)

El archivo Microsoft Excel ubicado en la red local SMB opera como un **Transactive Store** bidireccional. Para evitar condiciones de carrera (_Data Race_) o corrupción por bloqueos concurrentes de usuarios humanos, el Agente en Rust implementa un subsistema de aislamiento mediante exclusión mutua basada en el sistema de archivos de Windows 11.

##### 5.1. Mecanismo Antibloqueo (_Starvation Prevention Protocol_)

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

##### 5.2. Propiedades de Consistencia

| Propiedad                          | Garantía                                                                                                                                |
| :--------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------- |
| **Consistencia eventual perfecta** | Los datos jamás se pierden: la nube (BigQuery) es la fuente de verdad; SQLite es el buffer resiliente.                                  |
| **Cero corrupción de archivos**    | Nunca se escribe en Excel mientras el lock `~$` exista.                                                                                 |
| **Cero pérdida ante caída de red** | SQLite WAL persiste localmente. Upload a BigQuery se reintenta indefinidamente.                                                         |
| **Tablas dinámicas intactas**      | Las fórmulas y tablas dinámicas nativas de la `Hoja3` del Excel se recalculan limpiamente al cerrar y reabrir, sin intervención manual. |
| **No duplicación de archivos**     | No se generan "copias en conflicto" ni archivos temporales en la red SMB.                                                               |

##### 5.3. Diagrama de Flujo de Decisión

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

#### H4.2. PERSISTENCIA Y DATA MODEL

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

#### H4.3. CONTRACTS & INTERFACES

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

---

### [EXP-001] MÓDULO: EXPEDITION_STATE_MACHINE

**ESTADO:** PATCH_REVISION — Definición de matriz extendida FSM y componentes validadores institucionales de control.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-EXP-01·P3] MilestonePhaseFSMTransitions:**
  - **Desc:** Incorporar nuevos estados de validación institucional post-adjudicación y entrega física al ciclo de vida del expediente.
  - **Logic:** Tabla FSM completa post-parche v4.1:
    | Estado Actual | Evento Disparador | Estado Siguiente | Nota |
    |---|---|---|---|
    | `INITIATED` | `SCAN_STARTED` | `SCANNING` | |
    | `SCANNING` | `DRIVE_UPLOADED` (x2 docs) | `DOCS_CAPTURED` | |
    | `DOCS_CAPTURED` | `INFERENCE_STARTED` | `INFERENCE_PENDING` | |
    | `INFERENCE_PENDING` | `VALIDATION_PASSED` | `VALIDATED` | |
    | `VALIDATED` | `CATALOG_VALID` | `CATALOG_CHECKED` | |
    | `CATALOG_CHECKED` | `EMAIL_SENT` | `PENDING_PROCUREMENT_VERIFICATION` | |
    | `PENDING_PROCUREMENT_VERIFICATION` | `RESPONSE_RECEIVED` | `PROCEDENCIA_APROBADA` | |
    | `PROCEDENCIA_APROBADA` | `SUPPLIERS_ASSIGNED` | `ASIGNACION_PROVEEDORES` | |
    | `ASIGNACION_PROVEEDORES` | `QUOTATIONS_DISPATCHED` | `ESPERA_COTIZACIONES` | |
    | `ESPERA_COTIZACIONES` | `QUOTATION_RECEIVED` | _(sin cambio de estado)_ | Evento logged; matriz comparativa actualizada |
    | `ESPERA_COTIZACIONES` | `COMPARATIVE_MATRIX_CONSOLIDATED` | `CUADRO_COMPARATIVO_CONSOLIDADO` | Dispara cuando `valid_quotation_count >= 3` |
    | `ESPERA_COTIZACIONES` | `QUOTATION_DEADLINE_EXPIRED` | `COTIZACIONES_VENCIDAS` | |
    | `CUADRO_COMPARATIVO_CONSOLIDADO` | `AWARD_RECOMMENDATION_GENERATED` | `ADJUDICACION_SUGERIDA` | IA calcula menor precio conforme |
    | `ADJUDICACION_SUGERIDA` | `AWARD_CONFIRMED` | `ENVIADO_RECURSOS_FINANCIEROS` | Dispara hito institucional |
    | `ENVIADO_RECURSOS_FINANCIEROS` | `AUTORIZACION_SUBDIRECCION_RECEIVED` | `AUTORIZADO_SUBDIRECCION` | Cierre de hitos operativos |
    | `AUTORIZADO_SUBDIRECCION` | `USER_COMMIT` | `COMPLETED` | Cierre final del expediente |
    | `COTIZACIONES_VENCIDAS` | `USER_COMMIT_PARTIAL` | `COMPLETED` | |
    | `PROCEDENCIA_APROBADA` | `USER_COMMIT` | `COMPLETED` | Transición de escape heredada |
    | Cualquier estado activo | `VALIDATION_FAILED` | `REJECTED_VALIDATION_FAILED` | |
    | Cualquier estado activo | `CATALOG_INVALID` | `REJECTED_CATALOG_INACTIVE` | |
    | `PENDING_PROCUREMENT_VERIFICATION` | `PROCEDENCIA_DENEGADA` | `REJECTED_PROCUREMENT_DENIED` | |

    **Deprecaciones:**
    - `COTIZACIONES_RECIBIDAS` → **DEPRECATED**. Reemplazado por `CUADRO_COMPARATIVO_CONSOLIDADO`.
    - `ALL_QUOTATIONS_RECEIVED` → **DEPRECATED**. Reemplazado por `COMPARATIVE_MATRIX_CONSOLIDATED`.

  - **Post-Condition:** `Expedition.status` actualizado según transiciones. Los eventos deprecados son rechazados.

#### EXP-001 H4.1.1 — Matriz de Transiciones Extendida con Componente Validador

La tabla FSM existente documenta los estados y transiciones. La siguiente matriz extendida añade el **componente validador responsable** y la **acción del sistema** para cada transición, proporcionando la especificación completa de implementación.

```
[ INGESTADO ] ──► [ CATALOGO_VALIDADO ] ──► [ EN_COTIZACION ] ──► [ EVALUADO_GEMINI ]
                                                                       │
[ AUT_CAA ] ◄── [ RECURSOS_FINANCIEROS ] ◄── [ COMPARATIVO_CONSOLIDADO ] ◄┘
     │
     ▼
[ COMPLETED ]
```

| Estado Origen                      | Evento Detonante                     | Estado Destino                     | Componente Validador                  | Acción del Sistema                                                                                      |
| :--------------------------------- | :----------------------------------- | :--------------------------------- | :------------------------------------ | :------------------------------------------------------------------------------------------------------ |
| `void`                             | Carga de Oficio / Escaneo            | `INITIATED` → `SCANNING`           | Frontend App (Apps Script) + SCAN-001 | Inicializa expediente. Asigna `folio_dsa`. Dispara `ScannerBridge`.                                     |
| `SCANNING`                         | `DRIVE_UPLOADED` (x2 docs)           | `DOCS_CAPTURED`                    | SCAN-001 + Drive API                  | Valida que ambos documentos (Oficio + Negativa) estén en Drive.                                         |
| `DOCS_CAPTURED`                    | `INFERENCE_STARTED`                  | `INFERENCE_PENDING`                | AI-001 (Gemini 1.5 Flash)             | Envía ambos PDFs como Contexto Unificado. Extrae entidades y valida cruzadamente.                       |
| `INFERENCE_PENDING`                | `VALIDATION_PASSED`                  | `VALIDATED`                        | AI-001 + ValidationResult             | Si `items_match == true` AND `dates_consistent == true` → avanza. Si no → `REJECTED_VALIDATION_FAILED`. |
| `VALIDATED`                        | `CATALOG_VALID`                      | `CATALOG_CHECKED`                  | CAT-001 + BigQuery Cache              | Búsqueda $O(1)$ contra caché. Verifica código activo. Sugiere proveedores históricos.                   |
| `CATALOG_CHECKED`                  | `EMAIL_SENT`                         | `PENDING_PROCUREMENT_VERIFICATION` | MAIL-001 + Gmail API                  | Genera tracking token SHA256. Envía correo a Coordinación de Adquisiciones.                             |
| `PENDING_PROCUREMENT_VERIFICATION` | `RESPONSE_RECEIVED`                  | `PROCEDENCIA_APROBADA`             | MAIL-001 (polling Gmail 15 min)       | Intercepta respuesta. Convierte hilo a PDF. Parsea semánticamente (APPROVED/DENIED).                    |
| `PROCEDENCIA_APROBADA`             | `SUPPLIERS_ASSIGNED`                 | `ASIGNACION_PROVEEDORES`           | QUOT-001 + STAT-001                   | Consulta BigQuery para proveedores directos. Si < 3, fallback a afinidad CONAC.                         |
| `ASIGNACION_PROVEEDORES`           | `QUOTATIONS_DISPATCHED`              | `ESPERA_COTIZACIONES`              | QUOT-001 (DocumentFactory + Gmail)    | Genera PDF por proveedor (clon + merge + SHA256 hash). Despacho DIRECT o DRAFT.                         |
| `ESPERA_COTIZACIONES`              | `QUOTATION_RECEIVED`                 | _(log-only, sin cambio)_           | INBOUND-001 (Gmail polling 10 min)    | Registra evento individual. Extrae PDF. Invoca validación Gemini. Actualiza matriz.                     |
| `ESPERA_COTIZACIONES`              | `COMPARATIVE_MATRIX_CONSOLIDATED`    | `CUADRO_COMPARATIVO_CONSOLIDADO`   | COMP-001 + Apps Script Engine         | Al contar ≥ 3 cotizaciones validadas: congela hoja Sheets, calcula menor precio.                        |
| `ESPERA_COTIZACIONES`              | `QUOTATION_DEADLINE_EXPIRED`         | `COTIZACIONES_VENCIDAS`            | EXP-001 (Time trigger 5 días hábiles) | Marca vencimiento. Permite cierre parcial.                                                              |
| `CUADRO_COMPARATIVO_CONSOLIDADO`   | `AWARD_RECOMMENDATION_GENERATED`     | `ADJUDICACION_SUGERIDA`            | COMP-001 (AwardCalculator)            | Genera tarjeta de recomendación. Presenta al operador.                                                  |
| `ADJUDICACION_SUGERIDA`            | `AWARD_CONFIRMED`                    | `ENVIADO_RECURSOS_FINANCIEROS`     | Operador Humano + EXP-001             | Confirmación explícita. Exporta PDF + XLSX. Deposita en Drive/SMB. Dispara hito SUPRE.                  |
| `ENVIADO_RECURSOS_FINANCIEROS`     | `AUTORIZACION_SUBDIRECCION_RECEIVED` | `AUTORIZADO_SUBDIRECCION`          | Operador Humano + EXP-001             | Registro de folio_supre y fecha_supre en Bloque 3 del Ledger.                                           |
| `AUTORIZADO_SUBDIRECCION`          | `USER_COMMIT`                        | `COMPLETED`                        | Operador Humano + EXP-001             | Commit final. Persiste fila en Sheets/BigQuery/Excel. Puebla Bloques 4 y 5.                             |
| Cualquier activo                   | `VALIDATION_FAILED`                  | `REJECTED_VALIDATION_FAILED`       | AI-001                                | Bloqueo irreversible del expediente.                                                                    |
| Cualquier activo                   | `CATALOG_INVALID`                    | `REJECTED_CATALOG_INACTIVE`        | CAT-001                               | Bloqueo irreversible.                                                                                   |
| `PENDING_PROCUREMENT_VERIFICATION` | `PROCEDENCIA_DENEGADA`               | `REJECTED_PROCUREMENT_DENIED`      | MAIL-001                              | Bloqueo irreversible.                                                                                   |

**Invariantes FSM:**

- Ningún estado terminal (`REJECTED_*`, `COMPLETED`) acepta transiciones de salida.
- `QUOTATION_RECEIVED` es un evento de **log-only**: inserta en `ExpeditionEvent` pero no muta `Expedition.status`.
- `COMPARATIVE_MATRIX_CONSOLIDATED` es el **único** evento que fuerza la transición desde `ESPERA_COTIZACIONES` a un estado macro diferente.
- Los campos de Bloques 3, 4 y 5 del `FondoRevolventeLedger` solo son editables bajo los estados autorizados por `[ID-REQ-EXP-10]`.

- **[ID-REQ-EXP-01] EventSourcingAppendOnlyLog:**
  - **Desc:** Registrar cada acción del sistema como un evento inmutable en una hoja dedicada de Google Sheets (`Events`).
  - **Logic:** `INSERT INTO Events(expedition_id, event_type, payload, actor, timestamp)`.

- **[ID-REQ-EXP-03] TimelineReconstruction:**
  - **Desc:** Reconstruir la línea de tiempo visual proyectando eventos ordenados por `timestamp`.
  - **Post-Condition:** UI renderiza timeline cronológico con actor e identidad verificado.

- **[ID-REQ-EXP-04] UserCommitConfirmation:**
  - **Desc:** Requerir confirmación humana antes de persistir la transacción definitiva y cerrar el expediente.
  - **Logic:** `IF (status == PROCEDENCIA_APROBADA OR status == AUTORIZADO_SUBDIRECCION OR status == COTIZACIONES_VENCIDAS) AND user_confirms THEN COMMIT`.

- **[ID-REQ-EXP-05] NormativeDeadlineCalculation:**
  - **Desc:** Calcular plazos máximos por fase basándose en reglas de negocio y emitir alertas visuales.
  - **Logic:**
    ```
    deadlineConfig = {
      ESPERA_COTIZACIONES: { days: 5 },
      PENDING_PROCUREMENT_VERIFICATION: { days: 3 },
      CUADRO_COMPARATIVO_CONSOLIDADO: { days: 2 },
      ADJUDICACION_SUGERIDA: { days: 1 }
    }
    ```

- **[ID-REQ-EXP-06] ColorCodedTimelineRendering:**
  - **Desc:** Asignar clases de color según `deadline_status` (`DEADLINE_OK` -> verde, `DEADLINE_WARNING` -> amarillo, `DEADLINE_EXPIRED` -> rojo).

- **[ID-REQ-EXP-07] OperatorEmailEnrichment:**
  - **Desc:** Poblado automático del autor del evento usando el `operator_email` verificado.

- **[ID-REQ-EXP-08] IndividualQuotationEventLogging:**
  - **Desc:** Registrar de forma granular la recepción de cada cotización individual como evento de log-only, sin cambiar de fase macro.

- **[ID-REQ-EXP-09] MatrixConsolidationThreshold:**
  - **Desc:** Forzar FSM → `CUADRO_COMPARATIVO_CONSOLIDADO` y lockear celdas en Sheets al contar con $\ge 3$ cotizaciones validadas técnicas.

- **[ID-REQ-EXP-10] MilestonePopulationValidation:**
  - **Desc:** Validar que las celdas del ledger `FondoRevolventeLedger` de los Bloques 3, 4 y 5 se pueblen únicamente bajo los estados del ciclo de vida autorizados, previniendo incoherencias transaccionales.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `Expedition`
  - **Properties:** `{id: UUID, folio_code: String, status: ExpeditionStatusEnum, created_by: String, created_at: ISO8601, updated_at: ISO8601}`
  - **Constraints:** PK(`id`), UNIQUE(`folio_code`), NOT NULL(`status`, `folio_code`)

- **ENTITY:** `ExpeditionEvent`
  - **Properties:** `{id: UUID, expedition_id: UUID, event_type: EventTypeEnum, actor: String, payload: JSON, timestamp: ISO8601, deadline_status: DeadlineStatusEnum?}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`event_type`, `timestamp`)

- **ENTITY:** `DeadlineConfig`
  - **Properties:** `{id: UUID, phase_state: ExpeditionStatusEnum, business_days: Int32, is_active: Boolean}`
  - **Constraints:** PK(`id`), UNIQUE(`phase_state`)

- **ENUM:** `ExpeditionStatusEnum` = `[INITIATED, SCANNING, DOCS_CAPTURED, INFERENCE_PENDING, VALIDATED, CATALOG_CHECKED, PENDING_PROCUREMENT_VERIFICATION, PROCEDENCIA_APROBADA, ASIGNACION_PROVEEDORES, ESPERA_COTIZACIONES, CUADRO_COMPARATIVO_CONSOLIDADO, ADJUDICACION_SUGERIDA, ENVIADO_RECURSOS_FINANCIEROS, AUTORIZADO_SUBDIRECCION, REJECTED_VALIDATION_FAILED, REJECTED_CATALOG_INACTIVE, REJECTED_PROCUREMENT_DENIED, COMPLETED, COTIZACIONES_RECIBIDAS, COTIZACIONES_VENCIDAS]`

- **ENUM:** `EventTypeEnum` = `[CREATED, SCAN_STARTED, SCAN_COMPLETED, DRIVE_UPLOADED, INFERENCE_STARTED, INFERENCE_COMPLETED, VALIDATION_PASSED, VALIDATION_FAILED, CATALOG_VALID, CATALOG_INVALID, EMAIL_SENT, RESPONSE_RECEIVED, PROCEDENCIA_APROBADA, PROCEDENCIA_DENEGADA, USER_COMMIT, STATE_TRANSITION, SUPPLIERS_ASSIGNED, QUOTATIONS_DISPATCHED, QUOTATION_RECEIVED, ALL_QUOTATIONS_RECEIVED, QUOTATION_DEADLINE_EXPIRED, USER_COMMIT_PARTIAL, QUOTATION_VALIDATED, QUOTATION_REJECTED_NORMATIVE, COMPARATIVE_MATRIX_CONSOLIDATED, AWARD_RECOMMENDATION_GENERATED, AWARD_CONFIRMED, DRAFTS_CREATED, AUTORIZACION_SUBDIRECCION_RECEIVED, SUPRE_ASSIGNED, CAA_PACKAGE_SENT, CAA_AUTHORIZED, ORDER_ISSUED, DELIVERY_RECEIVED, INVOICE_REGISTERED, PAYMENT_COMPLETED]`

- **ENUM:** `DeadlineStatusEnum` = `[DEADLINE_OK, DEADLINE_WARNING, DEADLINE_EXPIRED]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `FSMEngine` | **TRIGGER:** Any domain event
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, current_status: ExpeditionStatusEnum, triggering_event: EventTypeEnum, operator_email: String, payload: JSON?}`
  - **DATA_CONTRACT (Output):** `{transitioned: Boolean, new_status: ExpeditionStatusEnum, deadline_status: DeadlineStatusEnum?, validation_errors: Array<String>?}`

- **COMPONENT:** `MilestoneValidator` | **TRIGGER:** Mutation attempt on Bloque 3, 4, 5 fields
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, current_status: ExpeditionStatusEnum, target_block: Int32, fields: Map<String, Any>}`
  - **DATA_CONTRACT (Output):** `{allowed: Boolean, violations: Array<String>?}`

---

### [CAT-001] MÓDULO: CATALOG_CACHE_SERVICE

**ESTADO:** PATCH_REVISION — Queries BigQuery en Apps Script actualizadas para consumir de las tablas reales dimensionales del Data Warehouse.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-CAT-01·P] BigQueryCatalogWarmup:**
  - **Desc:** Carga del catálogo analítico institucional y del historial de compras directo desde BigQuery a `CacheService` al inicio de la sesión, consumiendo el Data Warehouse dimensional de `[DW-001]`.
  - **Logic:**
    ```javascript
    function consultarHistoricoInsumo(codigoInsumo) {
      const projectId = "hospital-civil-4562";
      const request = {
        query: `
          SELECT
            d.razon_social AS proveedor,
            d.rfc_proveedor,
            r.unidad_medida,
            MIN(r.precio_unitario) AS precio_minimo,
            AVG(r.precio_unitario) AS precio_promedio,
            MAX(r.fecha_sistema) AS ultima_compra,
            COUNT(*) AS total_compras
          FROM \`${projectId}.hospital_civil.fact_recepciones_historicas\` r
          LEFT JOIN \`${projectId}.hospital_civil.dim_proveedores\` d
            ON r.proveedor_pk = d.proveedor_pk
          WHERE r.codigo_insumo = @codigo
          GROUP BY d.razon_social, d.rfc_proveedor, r.unidad_medida
          ORDER BY COUNT(*) DESC
          LIMIT 5;
        `,
        useLegacySql: false,
        parameterMode: "NAMED",
        queryParameters: [
          {
            name: "codigo",
            parameterType: { type: "STRING" },
            parameterValue: { value: codigoInsumo },
          },
        ],
      };
      const queryResults = BigQuery.Jobs.query(request, projectId);
      return queryResults.rows;
    }
    ```
  - **Post-Condition:** `CacheService` poblado con datos del DWH dimensional.

- **[ID-REQ-CAT-01·P2] FreeTierCompliance:**
  - **Desc:** Garantizar operaciones interactivas gratuitas mediante `Jobs.query` sin streaming de cuotas.

- **[ID-REQ-CAT-02] ItemCodeLookup:**
  - **Desc:** Verificar existencia y estado del insumo de 10 dígitos (LPAD normalizado en ETL) contra caché analítica.

- **[ID-REQ-CAT-03] HistoricalSupplierSuggestion:**
  - **Desc:** Sugerir proveedores cruzando insumos con el historial de recepciones en DWH para renderizarDropdown en UI.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `CatalogItem`
  - **Properties:** `{code: String(10), description: String, category: String, is_active: Boolean, unit_of_measure: String}`
  - **Constraints:** PK(`code`)
  - **Backing Store:** BigQuery table `hospital-civil-4562.inventario.catalogo_bienes`

- **ENTITY:** `PurchaseHistory` → **DEPRECATED** (subsumida por las consultas directas a hechos `fact_recepciones_historicas` y `fact_pedidos_historicos` en `[DW-001]`).

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `BigQueryClient` | **TRIGGER:** Cache miss
  - **DATA_CONTRACT (Input):** `{project_id: String, query: String, parameters: Array<JSON>}`
  - **DATA_CONTRACT (Output):**
    ```json
    {
      "rows": [
        {
          "proveedor": "2GOB, S. DE R.L. DE C.V.",
          "rfc_proveedor": "TGO120918XX1",
          "unidad_medida": "PIEZA",
          "precio_minimo": 812.0,
          "precio_promedio": 950.0,
          "ultima_compra": "2024-05-24",
          "total_compras": 15
        }
      ],
      "total_bytes_processed": 0,
      "job_complete": true,
      "source": "fact_recepciones_historicas + dim_proveedores"
    }
    ```

- **COMPONENT:** `CatalogLookupService` | **TRIGGER:** UI change on `item_code`
  - **DATA_CONTRACT (Input):** `{item_code: String(10)}`
  - **DATA_CONTRACT (Output):** `{found: Boolean, is_active: Boolean, item: CatalogItem?, suggested_suppliers: Array<JSON>}`

---

### [MAIL-001] MÓDULO: ASYNC_EMAIL_INTERCEPTION

**ESTADO:** UNCHANGED

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-MAIL-01] TrackingIdInjection:**
  - **Desc:** Generar e inyectar en metadatos el identificador `tracking_token` criptográfico y asunto con patrón de folio.
  - **Logic:** `tracking_token = SHA256(expedition_id + timestamp + salt)[0:16]`.

- **[ID-REQ-MAIL-02] GmailPollingTrigger:**
  - **Desc:** Trigger temporal para escanear correos entrantes que correspondan a la referencia de expedientes activos.

- **[ID-REQ-MAIL-03] ThreadPdfCapture:**
  - **Desc:** Almacenar el PDF de la cadena de correos en Drive, mapeando `response_pdf_drive_id`.

- **[ID-REQ-MAIL-04] ResponseSemanticParsing:**
  - **Desc:** Analizar cuerpo semánticamente para clasificar respuesta como `APPROVED` o `DENIED`.

- **[ID-REQ-MAIL-05] SilentUINotification:**
  - **Desc:** Almacenar fila de actualización visual en Sheets con `is_read = false`.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `EmailTracking`
  - **Properties:** `{id: UUID, expedition_id: UUID, tracking_token: String(16), subject: String, recipient_email: String, sent_at: ISO8601, responded_at: ISO8601, response_pdf_drive_id: String, parsed_decision: EmailDecisionEnum, status: EmailStatusEnum}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), UNIQUE(`tracking_token`)

- **ENTITY:** `Notification`
  - **Properties:** `{id: UUID, expedition_id: UUID, recipient_user_id: String, message: String, is_read: Boolean, created_at: ISO8601}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`)

- **ENUM:** `EmailStatusEnum` = `[SENT, AWAITING_RESPONSE, RESPONSE_RECEIVED, PARSED, TIMED_OUT]`
- **ENUM:** `EmailDecisionEnum` = `[APPROVED, DENIED, MANUAL_REVIEW_REQUIRED]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `EmailDispatcher` | **TRIGGER:** FSM transition to `CATALOG_CHECKED`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, folio_code: String, item_code: String, item_description: String, recipient_email: String, body_template: String}`
  - **DATA_CONTRACT (Output):** `{email_message_id: String, tracking_token: String(16), thread_id: String}`

- **COMPONENT:** `GmailPollingWorker` | **TRIGGER:** Apps Script Time-driven trigger (15 min)
  - **DATA_CONTRACT (Input):** `{search_query: String, known_tracking_tokens: Array<String>}`
  - **DATA_CONTRACT (Output):** `{matched_threads: Array<JSON>}`

---

### [ETL-001] MÓDULO: LEGACY_CSV_INGESTION_PIPELINE

**ESTADO:** PATCH_REVISION — Column mapping real desde headers de xfarma para compras (13 cols) y pedidos (18 cols). Date anomaly detection y exclusión de RFCs nulos. LPAD de insumos.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-ETL-01] FilesystemEventCapture:**
  - **Desc:** Watcher de filesystem (`notify`) para encolar la aparición de nuevos archivos CSV legacy.

- **[ID-REQ-ETL-02·P3] RealColumnMappingComprasLimpio:**
  - **Desc:** Mapear de forma estructurada las 13 columnas reales del archivo `compras_limpio.csv` (222,201 registros) a la tabla de hechos BQ `fact_recepciones_historicas`, parseando la fecha del sistema (`mov_fecha_sys`) y extrayendo `siniva` limpio.

- **[ID-REQ-ETL-06] PedidosCsvIngestion:**
  - **Desc:** Pipeline dedicado para procesar `pedidos.csv` (133,779 registros, 18 columnas) que: (a) convierte fecha de formato `DD/MM/YYYY`, (b) ignora las 10 columnas vacías nulas de staging (`atributo_portal` a `subfamilia`), (c) realiza limpieza de `nif` (RFC) descartando registros con valor vacío, (d) normaliza strings y realiza LPAD a 10.
  - **Logic:**
    ```rust
    let excluded_columns = vec![
        "atributo_portal", "familia_terap", "subfam_terap",
        "grupo_terap", "principio_act", "grupo", "subgrupo",
        "familia", "subfamilia"
    ];
    // Filtrar RFCs nulos
    let valid_records = records.into_iter()
        .filter(|r| !r.nif.trim().is_empty())
        .collect();
    ```
  - **Post-Condition:** `fact_pedidos_historicos` cargada con ~132,987 filas; 792 filas nulas registradas en log de anomalías.

- **[ID-REQ-ETL-07] DateAnomalyDetection:**
  - **Desc:** Detectar y filtrar fechas imposibles legacy (ej. `1900-01-01` como indicador de fecha vacía), seteando el campo a NULL en base de datos para no distorsionar las queries de recencia y reportando la anomalía en log.
  - **Logic:** `IF fecha < '1990-01-01' THEN fecha = NULL AND LogAnomaly(DATE_IMPOSSIBLE)`.

- **[ID-REQ-ETL-08] CodigoInsumoPadding:**
  - **Desc:** Forzar a 10 dígitos exactos agregando ceros a la izquierda en todos los códigos de insumo de xfarma, asegurando FKs íntegras.
  - **Logic:** `LPAD(TRIM(codigo), 10, '0')`.

- **[ID-REQ-ETL-09] DualFileDetection:**
  - **Desc:** Watcher bifurcado que identifica de forma separada `compras_limpio.csv` vs `pedidos.csv` y detona pipelines con column mappings específicos independientes.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `IngestionJob`
  - **Properties:** `{id: UUID, source_file_path: String, total_rows: Int64, processed_rows: Int64, error_rows: Int64, status: IngestionJobStatusEnum, started_at: ISO8601, completed_at: ISO8601?, source_file_type: SourceFileTypeEnum, rows_skipped_null_rfc: Int64?, columns_excluded: Int32?, anomalies_detected: Int64}`

- **ENTITY:** `CSVParseError`
  - **Properties:** `{id: UUID, job_id: UUID, line_number: Int64, raw_value: String, error_message: String, created_at: ISO8601}`

- **ENUM:** `SourceFileTypeEnum` = `[COMPRAS_LIMPIO, PEDIDOS, COMPRAS_RAW, FONDO_REVOLVENTE_LEDGER]`
- **ENUM:** `IngestionJobStatusEnum` = `[QUEUED, PARSING, INSERTING, UPLOADING_BQ, COMPLETED, FAILED]`

#### COLUMN MAPPING TABLE — `compras_limpio.csv` → `fact_recepciones_historicas`

| Columna CSV        | Tipo CSV     | Campo Destino         | Tipo BQ    | Transformación                                       |
| ------------------ | ------------ | --------------------- | ---------- | ---------------------------------------------------- |
| `id_registro`      | INT          | `id_registro`         | INT64      | Directo                                              |
| `mov_fecha_sys`    | `YYYY-MM-DD` | `fecha_sistema`       | DATE       | `PARSE_DATE`; si < `1990-01-01` → NULL + anomaly log |
| `mov_fecha_alb`    | `YYYY-MM-DD` | `fecha_albaran`       | DATE       | `PARSE_DATE`; nullable                               |
| `mov_ejercicio`    | INT          | `ejercicio_fiscal`    | INT64      | Directo                                              |
| `fk_codigo`        | STRING       | `codigo_insumo`       | STRING(10) | `LPAD(TRIM(), 10, '0')`                              |
| `descripcion`      | STRING       | `descripcion`         | STRING     | `TRIM()`                                             |
| `mov_cantidad`     | NUMERIC      | `cantidad_ingresada`  | NUMERIC    | Cast; reject negatives                               |
| `mov_precio_lin`   | NUMERIC      | `precio_unitario`     | NUMERIC    | Cast                                                 |
| `mov_impor_lin`    | NUMERIC      | `importe_total`       | NUMERIC    | Cast                                                 |
| `siniva`           | NUMERIC      | `precio_sin_iva`      | NUMERIC    | Cast; validate < `importe_total`                     |
| `proveedor_pk`     | INT          | `proveedor_pk`        | INT64      | Directo                                              |
| `proveedor_nombre` | STRING       | _(→ dim_proveedores)_ | —          | Join para enriquecer dimensión                       |
| `almacen_deno`     | STRING       | `almacen_destino`     | STRING     | `TRIM()`                                             |

#### COLUMN MAPPING TABLE — `pedidos.csv` → `fact_pedidos_historicos`

| Columna CSV       | Tipo CSV     | Campo Destino            | Tipo BQ    | Transformación                                       |
| ----------------- | ------------ | ------------------------ | ---------- | ---------------------------------------------------- |
| `nro_pedido`      | INT          | `numero_pedido`          | STRING     | `CAST AS STRING`                                     |
| `fecha`           | `DD/MM/YYYY` | `fecha_pedido`           | DATE       | `PARSE_DATE('%d/%m/%Y')`                             |
| `proveedor`       | STRING       | `razon_social_proveedor` | STRING     | `TRIM()`                                             |
| `nif`             | STRING       | `rfc_proveedor`          | STRING     | `TRIM()`; si NULL/empty → **SKIP ROW** + anomaly log |
| `codigo`          | STRING(10)   | `codigo_insumo`          | STRING(10) | `LPAD(TRIM(), 10, '0')`                              |
| `articulo`        | STRING       | `descripcion`            | STRING     | `TRIM()`                                             |
| `cantidad`        | NUMERIC      | `cantidad_pedida`        | NUMERIC    | Cast; reject negatives                               |
| `precio`          | NUMERIC      | `precio_con_iva`         | NUMERIC    | Cast                                                 |
| `precio_sin_iva`  | NUMERIC      | `precio_sin_iva`         | NUMERIC    | Cast                                                 |
| `atributo_portal` | —            | **EXCLUIDA**             | —          | 100% nula (133,779/133,779)                          |
| `familia_terap`   | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `subfam_terap`    | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `grupo_terap`     | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `principio_act`   | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `grupo`           | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `subgrupo`        | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `familia`         | —            | **EXCLUIDA**             | —          | 100% nula                                            |
| `subfamilia`      | —            | **EXCLUIDA**             | —          | 100% nula                                            |

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `CSVParserEngine` | **TRIGGER:** Ingestion job starting
  - **DATA_CONTRACT (Input):**
    ```json
    {
      "file_path": "C:\\HCG_Legacy_Exports\\pedidos.csv",
      "file_type": "PEDIDOS",
      "chunk_size": 5000,
      "column_mapping": {
        "nro_pedido": "numero_pedido",
        "fecha": { "target": "fecha_pedido", "format": "DD/MM/YYYY" },
        "nif": {
          "target": "rfc_proveedor",
          "trim": true,
          "null_action": "EXCLUDE_ROW"
        },
        "codigo": { "target": "codigo_insumo", "pad_left": 10 },
        "cantidad": "cantidad_pedida"
      },
      "excluded_columns": [
        "atributo_portal",
        "familia_terap",
        "subfam_terap",
        "grupo_terap",
        "principio_act",
        "grupo",
        "subgrupo",
        "familia",
        "subfamilia"
      ]
    }
    ```
  - **DATA_CONTRACT (Output):** `{job_id: UUID, file_type: SourceFileTypeEnum, parsed_records: Int64, skipped_null_rfc: Int64, anomalies: Array<Anomaly>, ready_for_load: Boolean}`

---

### [PROXY-001] MÓDULO: INTRANET_SCRAPING_PROXY

**ESTADO:** UNCHANGED

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-PROXY-01] ReverseSignalChannelSetup:**
  - **Desc:** Polling periódico asíncrono desde el Rust Agent sobre la tabla control sin puertos entrantes.
  - **Logic:** `SELECT * FROM scraping_requests WHERE status = 'PENDING' ORDER BY requested_at LIMIT 10`.

- **[ID-REQ-PROXY-02] IntranetHTTPRequest:**
  - **Desc:** Solicitud HTTP con `reqwest` y timeout de 10s contra la intranet.

- **[ID-REQ-PROXY-03] HTMLSemanticParsing:**
  - **Desc:** Parseo del DOM con crate `scraper` buscando clase CSS `.estatus-contrato`.

- **[ID-REQ-PROXY-04] ResponseChannelWriteback:**
  - **Desc:** Guardado de la respuesta inyectando estatus en `contract_status` y snapshot en `raw_html_snapshot`.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `ScrapingRequest`
  - **Properties:** `{id: UUID, expedition_id: UUID, item_code: String(10), status: ScrapingStatusEnum, contract_status: ContractStatusEnum?, raw_html_snapshot: String?, retry_count: Int32, error_message: String?, requested_at: ISO8601, responded_at: ISO8601?}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`item_code`, `status`, `requested_at`)

- **ENUM:** `ScrapingStatusEnum` = `[PENDING, IN_PROGRESS, COMPLETED, FAILED, RETRYING]`
- **ENUM:** `ContractStatusEnum` = `[VIGENTE, EN_PROCESO, SIN_CONTRATO, DESCONOCIDO]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `ScrapingRequestWriter` | **TRIGGER:** FSM transition in Apps Script
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, item_code: String(10)}`
  - **DATA_CONTRACT (Output):** `{request_id: UUID, status: "PENDING"}`

---

### [AUTH-001] MÓDULO: ZERO_PASSWORD_ACCESS_CONTROL

**ESTADO:** PATCH_REVISION — Integración de logs de acceso asíncronos y Access Gate federado.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-AUTH-01] FederatedIdentityInterception:**
  - **Desc:** Capturar de forma transparente `Session.getActiveUser().getEmail()` inyectando `operator_email`.

- **[ID-REQ-AUTH-02] WhitelistCacheValidation:**
  - **Desc:** Validar email contra la whitelist autorizada en cache con TTL de 6h.

- **[ID-REQ-AUTH-03] SessionContextInjection:**
  - **Desc:** Inyectar el `operator_email` en las propiedades de ejecución de Apps Script.

- **[ID-REQ-AUTH-04] AuditLogging:**
  - **Desc:** Registrar cada intento de acceso (exitoso o denegado) en la entidad `AccessAuditLog` con timestamp, email, resultado e IP del cliente (si disponible desde `request`).
  - **Logic:** Operación asíncrona, no bloqueante para el flujo principal.
    ```sql
    INSERT INTO AccessAuditLog(id, email, result, client_ip, timestamp)
    VALUES (UUID(), email, result, clientIp, NOW())
    ```
  - **Post-Condition:** Fila insertada. Intentos `DENIED_NOT_WHITELISTED` generan alerta en hoja `Control_Acceso` para revisión del administrador.

- **[ID-REQ-AUTH-05] ReferenceImplementation:**
  - **Desc:** Preservar la implementación de referencia del Access Gate para garantizar reproducibilidad.
  - **Logic:**

    ```javascript
    function evaluarPermisosAcceso() {
      const emailUsuario = Session.getActiveUser().getEmail();

      if (!emailUsuario) {
        registrarAcceso(emailUsuario, "DENIED_NO_IDENTITY");
        throw new Error("Acceso Denegado: Identidad no verificable.");
      }

      const cache = CacheService.getScriptCache();
      let listaBlanca = JSON.parse(cache.get("usuarios_autorizados"));

      if (!listaBlanca) {
        const sheet =
          SpreadsheetApp.openById(MASTER_SHEET_ID).getSheetByName(
            "Control_Acceso",
          );
        listaBlanca = sheet
          .getRange(2, 1, sheet.getLastRow() - 1, 1)
          .getValues()
          .flat();
        cache.put("usuarios_autorizados", JSON.stringify(listaBlanca), 21600);
      }

      if (listaBlanca.indexOf(emailUsuario) === -1) {
        registrarAcceso(emailUsuario, "DENIED_NOT_WHITELISTED");
        return { autorizado: false, email: emailUsuario };
      }

      registrarAcceso(emailUsuario, "GRANTED");
      return { autorizado: true, email: emailUsuario };
    }
    ```

  - **Post-Condition:** `AccessAuditLog` poblado en cada invocación.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `AccessControlEntry`
  - **Properties:** `{email: String, full_name: String, role: UserRoleEnum, is_active: Boolean, added_at: ISO8601}`
  - **Constraints:** PK(`email`), NOT NULL(`full_name`, `is_active`)

- **ENTITY:** `AccessAuditLog`
  - **Properties:** `{id: UUID, email: String, result: AccessResultEnum, client_ip: String?, timestamp: ISO8601}`
  - **Constraints:** PK(`id`), INDEX(`email`, `timestamp`)

- **ENUM:** `UserRoleEnum` = `[OPERADOR, SUPERVISOR, ADMIN]`
- **ENUM:** `AccessResultEnum` = `[GRANTED, DENIED_NOT_WHITELISTED, DENIED_NO_IDENTITY]`

---

### [QUOT-001] MÓDULO: QUOTATION_DOCUMENT_FACTORY

**ESTADO:** PATCH_REVISION — Definición de plantilla legal obligatoria de correo y despacho dinámico de cotizaciones.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-QUOT-01·P] BifurcatedSupplierResolution:**
  - **Desc:** Resolver proveedores mediante consulta analítica e invocar fallback a `[STAT-001]` de CONAC si no se alcanza la terna mínima.
  - **Logic:** `resolution_mode = [DIRECT, HYBRID]`, `dispatch_mode = [DIRECT, DRAFT]`.

- **[ID-REQ-QUOT-02] TemplateCloningAndMerge:**
  - **Desc:** Clonar plantilla de Google Docs e inyectar tokens `${PROVEEDOR_NOMBRE}` y `${FOLIO}`.

- **[ID-REQ-QUOT-03] DynamicItemTableInjection:**
  - **Desc:** Rellenar dinámicamente partidas en la tabla del documento.

- **[ID-REQ-QUOT-04] ImmutablePDFConversion:**
  - **Desc:** Generar PDF inmutable y destruir archivo editable.

- **[ID-REQ-QUOT-05] SHA256TraceabilityHash:**
  - **Desc:** Inyectar hash criptográfico `tracking_id` en pie de página del PDF.
  - **Logic:** `tracking_id = SHA256(folio_dsa + rfc_proveedor + timestamp)[0:16]`.

- **[ID-REQ-QUOT-06·P] DualModeDispatchPipeline:**
  - **Desc:** Despachar automáticamente emails directos (`DIRECT`) o crear borradores (`DRAFT`) en bandeja del operador para prevención de spam.
  - **TEMPLATE_HTML (cuerpo del correo de solicitud de cotización):**
    ```html
    <p>
      Estimado Representante Legal de
      <strong>${PROVEEDOR_RAZON_SOCIAL}</strong>,
    </p>
    <p>
      En apego al Artículo 13 de la Ley de Compras Gubernamentales,
      Enajenaciones y Contratación de Servicios del Estado de Jalisco, nos
      permitimos solicitar su valioso apoyo a efecto de que se realice la
      cotización para el estudio de mercado correspondiente al trámite de Fondo
      Revolvente con Folio <strong>DSA-${FOLIO}</strong>.
    </p>
    <p>
      Se adjunta a este correo el formato oficial con las especificaciones
      técnicas requeridas. Agradecemos que su respuesta cumpla estrictamente con
      los siguientes términos:
    </p>
    <ul>
      <li>Vigencia de cotización no menor a 30 días naturales.</li>
      <li>Garantías de calidad y caducidades desglosadas por partida.</li>
      <li>
        Remisión obligatoria del documento formalizado al correo:
        <strong>bcastro@hcg.gob.mx</strong>.
      </li>
    </ul>
    <p>
      Atentamente,<br />
      <strong>División de Servicios Administrativos</strong><br />
      Hospital Civil de Guadalajara
    </p>
    ```
  - **INVARIANT:** Este template es un activo de compliance normativo. Cualquier modificación debe ser aprobada por el área legal del HCG. Versionado obligatorio.

- **[ID-REQ-QUOT-07] QuotationDeadlineTrigger:**
  - **Desc:** Trigger temporal de vencimiento de cotización al cabo de 5 días hábiles.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `QuotationAssignment`
  - **Properties:** `{id: UUID, expedition_id: UUID, assigned_at: ISO8601, assigned_by: String, supplier_count: Int32, status: AssignmentStatusEnum, resolution_mode: ResolutionModeEnum, direct_supplier_count: Int32, affinity_supplier_count: Int32}`

- **ENTITY:** `QuotationDocument`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_id: String, supplier_rfc: String, tracking_id: String(16), template_id: String, pdf_drive_id: String, generated_at: ISO8601, generated_by: String}`
  - **Constraints:** PK(`id`), UNIQUE(`tracking_id`)

- **ENTITY:** `QuotationDispatch`
  - **Properties:** `{id: UUID, quotation_doc_id: UUID, expedition_id: UUID, supplier_email: String, subject: String, gmail_message_id: String?, dispatched_at: ISO8601, dispatched_by: String}`

- **ENUM:** `AssignmentStatusEnum` = `[PENDING, ASSIGNED, DISPATCHED, PARTIALLY_RECEIVED, COMPLETED, EXPIRED]`
- **ENUM:** `ResolutionModeEnum` = `[DIRECT, HYBRID]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `SupplierAssigner`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, item_code: String(10), partida_conac: String(4), min_suppliers: Int32, operator_email: String}`
  - **DATA_CONTRACT (Output):**
    ```json
    {
      "assignment_id": "UUID",
      "direct_suppliers": [
        {
          "name": "Proveedor A",
          "rfc": "PRAA850101XXX",
          "email": "ventas@alfa.com",
          "dispatch_mode": "DIRECT"
        }
      ],
      "affinity_suppliers": [
        {
          "name": "Proveedor Gamma",
          "rfc": "PRCC750303ZZZ",
          "email": "ventas@gamma.com",
          "dispatch_mode": "DRAFT",
          "affinity_index": 0.72
        }
      ],
      "resolution_mode": "HYBRID",
      "total_assigned": 3
    }
    ```

---

### [STAT-001] MÓDULO: SUPPLIER_AFFINITY_PROJECTION

**ESTADO:** PATCH_REVISION — SQL analítico reescrito para explotar el Data Warehouse dimensional de `[DW-001]` (`fact_pedidos_historicos` + `dim_proveedores`) con enriquecimiento de precios pagados reales desde `fact_recepciones_historicas`.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-STAT-01] CONACPartidaMapping:**
  - **Desc:** Mapear el código de bien a código de partida CONAC resolviendo contra las tablas de hechos de recepciones y pedidos en BQ de forma determinista.

- **[ID-REQ-STAT-02·P] RealDataAffinityQuery:**
  - **Desc:** Reemplazar la query anterior por un cruce real entre la tabla de hechos de pedidos y la dimensión proveedores sobre la partida CONAC de 4 dígitos, calculando el índice de afinidad ($I_A$) real de volumen (40%) y recencia de contratación (60%).
  - **Logic:**
    ```sql
    WITH compras_recientes AS (
      SELECT
        p.rfc_proveedor,
        p.razon_social_proveedor AS razon_social,
        COUNT(DISTINCT p.numero_pedido) AS total_pedidos,
        MAX(p.fecha_pedido) AS ultima_fecha_pedido
      FROM `proyecto.hospital_civil.fact_pedidos_historicos` p
      WHERE
        SUBSTR(p.codigo_insumo, 1, 4) = @partida_conac_solicitada
        AND p.rfc_proveedor IS NOT NULL
      GROUP BY p.rfc_proveedor, p.razon_social_proveedor
    )
    SELECT
      rfc_proveedor,
      razon_social,
      total_pedidos,
      ultima_fecha_pedido,
      (total_pedidos * 0.4)
        + (SAFE_DIVIDE(
            1,
            DATE_DIFF(CURRENT_DATE(), ultima_fecha_pedido, DAY) + 1
          ) * 0.6) AS indice_afinidad
    FROM compras_recientes
    ORDER BY indice_afinidad DESC
    LIMIT 5;
    ```

- **[ID-REQ-STAT-03] DirectSupplierExclusion:**
  - **Desc:** Exclusión de proveedores históricos asignados directamente.

- **[ID-REQ-STAT-05] CrossTableEnrichment:**
  - **Desc:** Enriquecer el listado cruzando con `fact_recepciones_historicas` para inyectar el último precio unitario real pagado en almacén para ese insumo/proveedor.
  - **Logic:** ROW_NUMBER partition desc por `fecha_sistema`.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `SupplierAffinityScore`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_rfc: String, supplier_razon_social: String, supplier_email: String, partida_conac: String(4), total_adjudicaciones: Int32, ultima_compra: ISO8601, affinity_index: Float64, calculated_at: ISO8601, ultimo_precio_real: Decimal128?, total_recepciones: Int64, ultima_recepcion: ISO8601?}`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `AffinityProjectionEngine`
  - **DATA_CONTRACT (Input):** `{projectId: String, itemCode: String, partidaConac: String(4), excludeRfcs: Array<String>}`
  - **DATA_CONTRACT (Output):**
    ```json
    {
      "candidates": [
        {
          "rfc_proveedor": "GLI800213MA5",
          "razon_social": "GAS LICUADO, S.A. DE C.V.",
          "total_pedidos": 42,
          "ultima_fecha_pedido": "2026-03-15",
          "affinity_index": 17.2,
          "ultimo_precio_real": 9.5,
          "total_recepciones": 38,
          "ultima_recepcion": "2026-04-01"
        }
      ],
      "query_bytes_processed": 0,
      "within_free_tier": true
    }
    ```

---

### [INBOUND-001] MÓDULO: SUPPLIER_QUOTATION_INTERCEPTION

**ESTADO:** UNCHANGED

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-INBOUND-01] SupplierResponsePolling:**
  - **Desc:** Escaneo de correos no leídos en Gmail que coincidan con folio y provengan de un remitente al que se le solicitó cotización.
  - **Logic:** `GmailApp.search('subject:"DSA-FR-" is:unread')`.

- **[ID-REQ-INBOUND-02] AttachmentExtractionAndNormalization:**
  - **Desc:** Conversión de imágenes adjuntas a PDF y nombrado estandarizado.
  - **Logic:** `DSA-${folio}_COTIZACION_${supplier_rfc}_${timestamp}.pdf`.

- **[ID-REQ-INBOUND-03] BodyToPDFCapture:**
  - **Desc:** En ausencia de adjuntos, capturar cuerpo del correo convirtiendo hilo a PDF.

- **[ID-REQ-INBOUND-04] DuplicateReceiptPrevention:**
  - **Desc:** Idempotencia y protección usando `gmail_message_id` como clave única.

- **[ID-REQ-INBOUND-05] AutomaticGeminiValidationTrigger:**
  - **Desc:** Invocar síncronamente validación normativa en modulo `COMP-001`.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `QuotationResponse`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_rfc: String, supplier_email: String, gmail_message_id: String, gmail_thread_id: String, subject: String, pdf_drive_id: String, pdf_file_name: String, received_at: ISO8601, processed_at: ISO8601, processed_by: String}`
  - **Constraints:** PK(`id`), UNIQUE(`gmail_message_id`), FK(`expedition_id` → `Expedition.id`)

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `SupplierResponsePoller`
  - **DATA_CONTRACT (Input):** `{search_query: String, known_dispatched_suppliers: Map<String, Array<String>>}`
  - **DATA_CONTRACT (Output):** `{matched_threads: Array<JSON>}`

---

### [COMP-001] MÓDULO: COMPARATIVE_MATRIX_ENGINE

**ESTADO:** PATCH_REVISION — Normalización 3NF: `MatrixEntry` + `ComparativeMatrix` reemplazados por `EstudioMercadoMetadata` + `EstudioMercadoLineas`.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-COMP-01] NormativeQuotationValidation:**
  - **Desc:** Analizar semánticamente mediante Gemini 1.5 Flash cada cotización recibida frente a la solicitud original. El motor valida la vigencia de precios ($\ge 30$ días naturales), método de pago y anexo técnico.
  - **Logic:** Invocación multimodal pasándole el PDF. `responseSchema` en `snake_case`:
    ```json
    {
      "type": "object",
      "properties": {
        "vigencia_precios_dias": { "type": "integer" },
        "vigencia_cumple": { "type": "boolean" },
        "metodo_pago_aceptable": { "type": "boolean" },
        "anexo_tecnico_coincide": { "type": "boolean" },
        "unidad_ofrecida": { "type": "string" },
        "unidad_requerida": { "type": "string" },
        "precio_unitario_ofertado": { "type": "number" },
        "importe_total_ofertado": { "type": "number" },
        "tiempo_entrega_dias": { "type": "integer" },
        "tipo_dias": { "type": "string", "enum": ["NATURALES", "HABILES"] },
        "condiciones_pago": { "type": "string" },
        "cumple_anexo_tecnico": { "type": "boolean" },
        "motivo_rechazo_normativo": { "type": "string" },
        "estatus_validacion": {
          "type": "string",
          "enum": ["VALIDADO", "DEFICIENTE_NORMATIVAMENTE"]
        }
      },
      "required": [
        "vigencia_cumple",
        "metodo_pago_aceptable",
        "anexo_tecnico_coincide",
        "precio_unitario_ofertado",
        "importe_total_ofertado",
        "cumple_anexo_tecnico",
        "estatus_validacion"
      ]
    }
    ```

- **[ID-REQ-COMP-02·P] ThreeNFMatrixNormalization:**
  - **Desc:** Reemplazar `MatrixEntry` y `ComparativeMatrix` por un modelo en Tercera Forma Normal (3NF) segregando metadatos generales (`estudio_mercado_metadata`) de las ofertas individuales (`estudio_mercado_lineas`).
  - **Logic:** Insertar datos globales del estudio en metadata, y mapear cada partida cotizada por proveedor a las líneas desglosadas.
  - **Post-Condition:** Estructura normalizada sin redundancia persistida en Sheets y BigQuery.

- **[ID-REQ-COMP-03] MatrixLockOnConsolidation:**
  - **Desc:** Proteger la pestaña del Cuadro Comparativo en Sheets para evitar modificaciones de celdas cuando se logre la terna ($\ge 3$ cotizaciones validadas técnicas).

- **[ID-REQ-COMP-04] LowestCompliantBidCalculation:**
  - **Desc:** Identificar de forma automática la propuesta económica que presenta el menor costo total de entre las ofertas con estatus `VALIDADO`.
  - **Logic:**
    ```javascript
    const validEntries = lines.filter(
      (e) => e.estatus_validacion === "VALIDADO",
    );
    validEntries.sort(
      (a, b) => a.importe_total_ofertado - b.importe_total_ofertado,
    );
    const winner = validEntries[0];
    ```
  - **Post-Condition:** Registro `AwardRecommendation` persistido; la FSM transiciona a `ADJUDICACION_SUGERIDA`.

- **[ID-REQ-COMP-05] AwardRecommendationCard:**
  - **Desc:** Tarjeta interactiva en la UI que detalla la adjudicación sugerida, permitiendo la confirmación explícita del usuario inyectando `operator_email` a `AWARD_CONFIRMED`.
  - **WIREFRAME (tarjeta de recomendación de adjudicación):**
    ```
    ┌──────────────────────────────────────────────────────────┐
    │ ⚖️ RECOMENDACIÓN DE ADJUDICACIÓN SUGERIDA POR IA         │
    ├──────────────────────────────────────────────────────────┤
    │ Folio: DSA-${folio_dsa}                                  │
    │ Proveedor Sugerido: ${recommended_razon_social}          │
    │ Propuesta Económica: $${recommended_precio_total} MXN    │
    │   (La más baja entre conformes)                          │
    │ Dictamen Normativo: Cumple 100% Anexo Técnico            │
    │ Vigencia: ${recommended_vigencia_dias} días              │
    │   (Margen legal óptimo)                                  │
    ├──────────────────────────────────────────────────────────┤
    │ [ [ VALIDAR REGISTRO Y CONTRATAR ] ]                     │
    └──────────────────────────────────────────────────────────┘
    ```
  - **Nota de Gobernanza:** El botón de confirmación final permanece bajo la responsabilidad exclusiva del usuario humano, respetando la regla del operador único. La acción invoca `FSMEngine(expeditionId, AWARD_CONFIRMED, operator_email)`.

- **[ID-REQ-COMP-06] MultiformatExport:**
  - **Desc:** Tras la confirmación, exportar cuadro comparativo como PDF inmutable y XLSX, guardando en Drive y local SMB.
  - **REFERENCE_IMPLEMENTATION (exportación PDF + XLSX):**

    ```javascript
    function exportarCuadroComparativo(sheetId, folio, carpetaExpediente) {
      const oauthToken = ScriptApp.getOAuthToken();

      // Exportar como PDF
      const pdfUrl =
        "https://docs.google.com/spreadsheets/d/" +
        sheetId +
        "/export?format=pdf&gid=" +
        obtenerGid(sheetId);
      const pdfBlob = UrlFetchApp.fetch(pdfUrl, {
        headers: { Authorization: "Bearer " + oauthToken },
      })
        .getBlob()
        .setName("Cuadro_Comparativo_DSA_" + folio + ".pdf");
      const pdfFile = carpetaExpediente.createFile(pdfBlob);

      // Exportar como XLSX
      const xlsxUrl =
        "https://docs.google.com/spreadsheets/d/" +
        sheetId +
        "/export?format=xlsx";
      const xlsxBlob = UrlFetchApp.fetch(xlsxUrl, {
        headers: { Authorization: "Bearer " + oauthToken },
      })
        .getBlob()
        .setName("Cuadro_Comparativo_DSA_" + folio + ".xlsx");
      const xlsxFile = carpetaExpediente.createFile(xlsxBlob);

      return {
        pdf_drive_id: pdfFile.getId(),
        xlsx_drive_id: xlsxFile.getId(),
        smb_sync_pending: true,
      };
    }
    ```

- **[ID-REQ-COMP-07] RejectionAuditTrail:**
  - **Desc:** Auditar motivos de rechazo normativo en la bitácora de eventos del expediente.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `EstudioMercadoMetadata`
  - **Properties:** `{folio_dsa: String, fecha_estudio: Date, area_solicitante: String, articulo_ley_fundamento: String, is_locked: Boolean, exported_pdf_drive_id: String?, exported_xlsx_drive_id: String?, created_at: ISO8601, updated_at: ISO8601}`
  - **Constraints:** PK(`folio_dsa`)

- **ENTITY:** `EstudioMercadoLineas`
  - **Properties:** `{folio_dsa: String, proveedor_rfc: String, proveedor_padron_id: String?, proveedor_razon_social: String, tiempo_entrega_dias: Int64?, tipo_dias: String?, condiciones_pago: String?, correo_contacto: String?, precio_unitario_ofertado: Decimal128, importe_total_ofertado: Decimal128, moneda: String(3), cumple_anexo_tecnico: Boolean, motivo_rechazo_normativo: String?, estatus_validacion: String, quotation_response_id: UUID, gemini_raw_response: JSON, created_at: ISO8601}`
  - **Constraints:** PK(`folio_dsa`, `proveedor_rfc`), FK(`folio_dsa` → `EstudioMercadoMetadata.folio_dsa`), FK(`quotation_response_id` → `QuotationResponse.id`)

- **ENTITY:** `AwardRecommendation`
  - **Properties:** `{id: UUID, folio_dsa: String, recommended_proveedor_rfc: String, recommended_razon_social: String, recommended_precio_total: Decimal128, recommended_vigencia_dias: Int32, normative_compliance: Boolean, justification: String, generated_at: ISO8601, confirmed_by: String?, confirmed_at: ISO8601?}`
  - **Constraints:** PK(`id`), FK(`folio_dsa` → `EstudioMercadoMetadata.folio_dsa`)

- **ENUM:** `ValidationStatusEnum` = `[VALIDADO, DEFICIENTE_NORMATIVAMENTE, PENDING_VALIDATION]`

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `NormativeValidator` | **TRIGGER:** `QuotationResponse` persisted
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, pdf_drive_id: String, supplier_rfc: String, original_request: JSON}`
  - **DATA_CONTRACT (Output):** JSON compatible con `EstudioMercadoLineas`.

- **COMPONENT:** `MatrixConsolidator` | **TRIGGER:** Valid `EstudioMercadoLineas` count $\ge 3$
  - **DATA_CONTRACT (Input):** `{folio_dsa: String, threshold: Int32, operator_email: String}`
  - **DATA_CONTRACT (Output):** `{consolidated: Boolean, valid_count: Int32, is_locked: Boolean}`

- **COMPONENT:** `AwardCalculator` | **TRIGGER:** `COMPARATIVE_MATRIX_CONSOLIDATED` event
  - **DATA_CONTRACT (Input):** `{folio_dsa: String}`
  - **DATA_CONTRACT (Output):** JSON de estructura `AwardRecommendation`.

---

### [DW-001] MÓDULO: HISTORICAL_DATA_WAREHOUSE

**ESTADO:** PATCH_REVISION — Especificación de esquemas dimensionales extendidos de alta volumetría.

**Propósito:** Definir el modelo dimensional (esquema estrella) en Google BigQuery que consolidará los 355,980 registros históricos reales extraídos del sistema legacy xfarma/Dedalus del Hospital Civil de Guadalajara, siendo la **fuente de verdad analítica** consumida por los módulos STAT-001 y CAT-001.

#### H4.1. REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-DW-01] ProviderDimensionConstruction:**
  - **Desc:** Construir la dimensión `dim_proveedores` consolidando registros únicos desde `pedidos.csv` (fuente primaria, contiene RFC via `nif`) y cruzando con `compras_limpio.csv` (fuente secundaria, contiene `proveedor_pk` y `proveedor_nombre`) para asociar ID legacy con el RFC real.
  - **Logic:**

    ```sql
    -- Seed desde pedidos
    INSERT INTO dim_proveedores (rfc_proveedor, razon_social, fuente_primaria)
    SELECT DISTINCT TRIM(nif), TRIM(proveedor), 'PEDIDOS'
    FROM staging_pedidos WHERE nif IS NOT NULL AND TRIM(nif) != '';

    -- Enriquecimiento compras matching nombre
    UPDATE dim_proveedores d
    SET proveedor_pk = c.proveedor_pk
    FROM (SELECT DISTINCT proveedor_pk, proveedor_nombre FROM staging_compras) c
    WHERE d.razon_social = c.proveedor_nombre;
    ```

- **[ID-REQ-DW-02] RecepcionesFactTableDesign:**
  - **Desc:** Tabla de hechos `fact_recepciones_historicas` particionada por `fecha_sistema` (DATE) y clusterizada por `codigo_insumo` + `proveedor_pk`.
  - **Logic:** DDL verificado en DWH.

- **[ID-REQ-DW-03] PedidosFactTableDesign:**
  - **Desc:** Tabla de hechos `fact_pedidos_historicos` particionada por `fecha_pedido` (DATE) y clusterizada por `codigo_insumo` + `rfc_proveedor`, excluyendo las 10 columnas vacías nulas del CSV legacy.
  - **Logic:** DDL verificado en DWH.

- **[ID-REQ-DW-04] AnomalyDetectionRules:**
  - **Desc:** Tratamiento preventivo de anomalías en datos legacy: fechas vacías imposibles `1900-01-01` se setean a NULL, nifs nulos de pedidos se descartan en ETL, `siniva` corrupto de `compras_completo_corregido.csv` es filtrado, y códigos reciben LPAD a 10.

- **[ID-REQ-DW-05] ParticionamientoYFreeTier:**
  - **Desc:** Garantizar almacenamiento total del DWH analítico <200 MB, permaneciendo bajo Always Free Tier de BigQuery.

#### H4.2. PERSISTENCIA Y DATA MODEL

- **ENTITY:** `DimProveedor`
  - **Properties (BigQuery DDL):**
    ```sql
    CREATE TABLE `proyecto.hospital_civil.dim_proveedores` (
      rfc_proveedor     STRING    NOT NULL,    -- PK
      razon_social      STRING    NOT NULL,
      proveedor_pk      INT64,                 -- FK compras
      fuente_primaria   STRING    NOT NULL,
      nif_completo      STRING,
      registros_pedidos INT64,
      registros_compras INT64,
      primera_transaccion DATE,
      ultima_transaccion  DATE,
      created_at        TIMESTAMP NOT NULL,
      updated_at        TIMESTAMP NOT NULL
    );
    ```

- **ENTITY:** `FactRecepcionesHistoricas`
  - **Properties (BigQuery DDL):**
    ```sql
    CREATE TABLE `proyecto.hospital_civil.fact_recepciones_historicas` (
      id_registro           INT64       NOT NULL,
      fecha_sistema         DATE        NOT NULL,   -- Partición
      fecha_albaran         DATE,
      ejercicio_fiscal      INT64       NOT NULL,
      codigo_insumo         STRING(10)  NOT NULL,   -- Cluster
      descripcion           STRING      NOT NULL,
      cantidad_ingresada    NUMERIC     NOT NULL,
      precio_unitario       NUMERIC     NOT NULL,
      importe_total         NUMERIC     NOT NULL,
      precio_sin_iva        NUMERIC,
      proveedor_pk          INT64       NOT NULL,   -- Cluster
      almacen_destino       STRING      NOT NULL,
      created_at            TIMESTAMP   NOT NULL
    )
    PARTITION BY fecha_sistema
    CLUSTER BY codigo_insumo, proveedor_pk;
    ```

- **ENTITY:** `FactPedidosHistoricos`
  - **Properties (BigQuery DDL):**
    ```sql
    CREATE TABLE `proyecto.hospital_civil.fact_pedidos_historicos` (
      numero_pedido           STRING      NOT NULL,
      fecha_pedido            DATE        NOT NULL,  -- Partición
      rfc_proveedor           STRING      NOT NULL,  -- Cluster
      razon_social_proveedor  STRING      NOT NULL,
      codigo_insumo           STRING(10)  NOT NULL,  -- Cluster
      descripcion             STRING      NOT NULL,
      cantidad_pedida         NUMERIC     NOT NULL,
      precio_con_iva          NUMERIC     NOT NULL,
      precio_sin_iva          NUMERIC     NOT NULL,
      created_at              TIMESTAMP   NOT NULL
    )
    PARTITION BY fecha_pedido
    CLUSTER BY codigo_insumo, rfc_proveedor;
    ```

- **ENTITY:** `EtlAnomalyLog`
  - **Properties (BigQuery DDL):**
    ```sql
    CREATE TABLE `proyecto.hospital_civil.etl_anomaly_log` (
      id              STRING    NOT NULL,
      source_file     STRING    NOT NULL,
      row_number      INT64     NOT NULL,
      field_name      STRING    NOT NULL,
      raw_value       STRING,
      anomaly_type    STRING    NOT NULL,
      action_taken    STRING    NOT NULL,
      created_at      TIMESTAMP NOT NULL
    );
    ```

#### DW-001 H4.2.1 — Esquemas de Datos de Infraestructura de Alta Volumetría

Para dar soporte a la ingesta masiva del histórico institucional (~355,980 registros consolidados de xfarma y Dedalus), se implementa un modelo analítico columnar particionado en Google BigQuery (Cloud) acoplado a un motor in-memory transaccional en el Edge (SQLite via Rust).

##### Tabla A: `fact_recepciones_historicas` (Carga masiva desde `compras_limpio.csv`)

- **Engine:** Google BigQuery (Columnar Store)
- **Partitioning:** `PARTITION BY fecha_sistema` (Mensual)
- **Clustering:** `CLUSTER BY codigo_insumo, proveedor_pk`
- **Volumetría:** 222,201 registros \| ~111 MB estimado

| Nombre del Campo     | Tipo de Dato    | Restricción / Indexación                                    | Descripción                                                            |
| :------------------- | :-------------- | :---------------------------------------------------------- | :--------------------------------------------------------------------- |
| `id_registro`        | `INT64`         | PRIMARY KEY                                                 | Identificador único secuencial de la transacción.                      |
| `fecha_sistema`      | `DATE`          | REQUIRED, **PARTITION KEY**                                 | Timestamp de confirmación en el Kernel del ERP (`mov_fecha_sys`).      |
| `fecha_albaran`      | `DATE`          | NULLABLE                                                    | Fecha física del documento de entrega del proveedor (`mov_fecha_alb`). |
| `ejercicio_fiscal`   | `INT64`         | REQUIRED                                                    | Año de afectación contable (`mov_ejercicio`).                          |
| `codigo_insumo`      | `STRING(10)`    | REQUIRED, **CLUSTER KEY**                                   | Identificador numérico LPAD-normalizado (`fk_codigo`).                 |
| `partida_conac`      | `STRING(4)`     | REQUIRED                                                    | Derivado: `SUBSTR(codigo_insumo, 1, 4)`. Clasificador CONAC Nivel 4.   |
| `descripcion`        | `STRING`        | REQUIRED                                                    | Texto técnico del bien o servicio (`descripcion`).                     |
| `cantidad_ingresada` | `NUMERIC(15,4)` | REQUIRED                                                    | Cantidad física volumétrica recibida en almacén (`mov_cantidad`).      |
| `precio_unitario`    | `NUMERIC(15,4)` | REQUIRED                                                    | Costo unitario antes de impuestos (`mov_precio_lin`).                  |
| `importe_total`      | `NUMERIC(15,4)` | REQUIRED                                                    | Costo neto de la línea (`mov_impor_lin`).                              |
| `precio_sin_iva`     | `NUMERIC(15,4)` | NULLABLE                                                    | Precio sin IVA extraído de `siniva` (limpio en `compras_limpio.csv`).  |
| `proveedor_pk`       | `INT64`         | REQUIRED, **CLUSTER KEY**, FK semántica → `dim_proveedores` | Identificador foráneo del catálogo legacy.                             |
| `almacen_destino`    | `STRING`        | REQUIRED                                                    | Denominación del almacén receptor (ej. `ALMACEN GENERAL FAA`).         |
| `created_at`         | `TIMESTAMP`     | REQUIRED                                                    | Metadato de auditoría de carga.                                        |

**Invariantes:**

- `fecha_sistema` < `1990-01-01` → `NULL` + entrada en `etl_anomaly_log` (tipo: `DATE_IMPOSSIBLE`).
- `precio_sin_iva` > `importe_total` → registro descartado + anomaly log (tipo: `NUMERIC_OVERFLOW`).
- `cantidad_ingresada` < 0 → registro descartado + anomaly log.

##### Tabla B: `fact_pedidos_historicos` (Carga masiva desde `pedidos.csv`)

- **Engine:** Google BigQuery (Columnar Store)
- **Partitioning:** `PARTITION BY fecha_pedido` (Mensual)
- **Clustering:** `CLUSTER BY codigo_insumo, rfc_proveedor`
- **Volumetría:** ~132,987 registros (133,779 - 792 RFC nulos) \| ~53 MB estimado

| Nombre del Campo         | Tipo de Dato    | Restricción / Indexación    | Descripción                                                                   |
| :----------------------- | :-------------- | :-------------------------- | :---------------------------------------------------------------------------- |
| `numero_pedido`          | `STRING`        | PRIMARY KEY                 | Código del contrato o pedido asignado (`nro_pedido` cast a STRING).           |
| `fecha_pedido`           | `DATE`          | REQUIRED, **PARTITION KEY** | Fecha de formalización legal (`fecha`, parse `DD/MM/YYYY`).                   |
| `rfc_proveedor`          | `STRING`        | REQUIRED, **CLUSTER KEY**   | Registro Federal de Contribuyentes (`nif`, TRIM). Si nulo → **ROW EXCLUDED**. |
| `razon_social_proveedor` | `STRING`        | REQUIRED                    | Denominación comercial del licitante (`proveedor`, TRIM).                     |
| `codigo_insumo`          | `STRING(10)`    | REQUIRED, **CLUSTER KEY**   | Clave del catálogo institucional (`codigo`, LPAD 10).                         |
| `descripcion`            | `STRING`        | REQUIRED                    | Texto técnico del bien o servicio (`articulo`).                               |
| `cantidad_pedida`        | `NUMERIC(15,4)` | REQUIRED                    | Volumen total comprometido (`cantidad`).                                      |
| `precio_con_iva`         | `NUMERIC(15,4)` | REQUIRED                    | Costo unitario pactado con IVA (`precio`).                                    |
| `precio_sin_iva`         | `NUMERIC(15,4)` | REQUIRED                    | Costo unitario pactado sin IVA (`precio_sin_iva`).                            |
| `created_at`             | `TIMESTAMP`     | REQUIRED                    | Metadato de auditoría de carga.                                               |

**Columnas EXCLUIDAS del CSV (100% nulas en los 133,779 registros):**
`atributo_portal`, `familia_terap`, `subfam_terap`, `grupo_terap`, `principio_act`, `grupo`, `subgrupo`, `familia`, `subfamilia` (9 columnas eliminadas del schema de carga, reduciendo ancho de 18 a 9 campos útiles + 1 metadato).

##### Tabla C: `estudio_mercado_lineas` (Cuadro Comparativo Normalizado 3NF)

- **Engine:** Google Sheets (Cloud Interface) + BigQuery (replica analítica)
- **PK Compuesta:** `(folio_dsa, proveedor_rfc)`

| Nombre del Campo           | Tipo de Dato    | Restricción / Indexación                                                | Descripción                                                        |
| :------------------------- | :-------------- | :---------------------------------------------------------------------- | :----------------------------------------------------------------- |
| `folio_dsa`                | `STRING`        | PK, FK → `Expedition.folio_code`                                        | Identificador del expediente raíz.                                 |
| `proveedor_rfc`            | `STRING`        | PK                                                                      | RFC de la empresa evaluada.                                        |
| `proveedor_padron_id`      | `STRING`        | NULLABLE                                                                | Registro vigente en el padrón de proveedores HCG (ej. `P21221`).   |
| `proveedor_razon_social`   | `STRING`        | REQUIRED                                                                | Denominación comercial completa.                                   |
| `tiempo_entrega_dias`      | `INT64`         | NULLABLE                                                                | Plazo ofertado de suministro.                                      |
| `tipo_dias`                | `STRING`        | ENUM: `["NATURALES", "HABILES"]`                                        | Naturaleza del plazo.                                              |
| `condiciones_pago`         | `STRING`        | REQUIRED                                                                | Términos comerciales (ej. "30 días crédito").                      |
| `correo_contacto`          | `STRING`        | NULLABLE                                                                | Email del representante comercial.                                 |
| `precio_unitario_ofertado` | `NUMERIC(15,4)` | REQUIRED                                                                | Valor unitario de la oferta económica sin IVA.                     |
| `importe_total_ofertado`   | `NUMERIC(15,4)` | REQUIRED                                                                | Valor total de la oferta económica.                                |
| `moneda`                   | `STRING(3)`     | REQUIRED, DEFAULT `'MXN'`                                               | Código ISO 4217 de la moneda.                                      |
| `cumple_anexo_tecnico`     | `BOOLEAN`       | REQUIRED                                                                | Bandera de idoneidad determinada por Gemini y/o validación humana. |
| `estatus_validacion`       | `STRING`        | ENUM: `["VALIDADO", "DEFICIENTE_NORMATIVAMENTE", "PENDING_VALIDATION"]` | Resultado de la validación normativa.                              |
| `motivo_rechazo_normativo` | `STRING`        | NULLABLE                                                                | Bitácora de descalificación (ej. "Vigencia menor a 30 días").      |
| `quotation_response_id`    | `UUID`          | FK → `QuotationResponse.id`                                             | Enlace al PDF de la cotización original interceptada.              |
| `gemini_raw_response`      | `JSON`          | NULLABLE                                                                | Payload completo retornado por Gemini para auditoría.              |
| `created_at`               | `TIMESTAMP`     | REQUIRED                                                                | Metadato de auditoría.                                             |

**Invariantes:**

- `estatus_validacion == "VALIDADO"` requiere `cumple_anexo_tecnico == true` AND `precio_unitario_ofertado > 0`.
- Consolidación automática al contar `COUNT(*) WHERE estatus_validacion = 'VALIDADO' >= 3`.
- Una vez consolidada (`is_locked = true`), ninguna escritura programática es permitida en las celdas de datos.

#### H4.3. CONTRACTS & INTERFACES

- **COMPONENT:** `RecepcionesLoadJob` | **TRIGGER:** file compras_limpio.csv created
  - **DATA_CONTRACT (Input):** `{file_path: String, target_table: String}`
  - **DATA_CONTRACT (Output):** `{job_id: String, rows_loaded: Int64, anomalies_detected: Int64}`

---

## CONSOLIDACIÓN DE INTEGRIDAD REFERENCIAL (ESTADO POST-PATCH v5.1)

```
  DATA WAREHOUSE (BigQuery — DW-001):
  ┌─────────────────────────────┐
  │     dim_proveedores         │
  │─────────────────────────────│
  │ rfc_proveedor (PK)          │◄──────┐
  │ razon_social                │       │
  │ proveedor_pk                │──┐    │
  │ registros_pedidos           │  │    │
  │ registros_compras           │  │    │
  └─────────────────────────────┘  │    │
       ▲                            │    │
       │ JOIN                       │    │ FK (semántica)
       ▼                            │    │
  ┌─────────────────────────────┐  │    │
  │ fact_pedidos_historicos     │  │    │
  │─────────────────────────────│  │    │
  │ numero_pedido (PK)          │  │    │
  │ fecha_pedido (PARTITION)    │  │    │
  │ rfc_proveedor (FK, CLUSTER) │──┘    │
  │ codigo_insumo (CLUSTER)     │       │
  │ cantidad_pedida             │       │
  │ precio_con_iva              │       │
  │ precio_sin_iva              │       │
  └─────────────────────────────┘       │
                                        │
  ┌─────────────────────────────┐       │
  │ fact_recepciones_historicas │       │
  │─────────────────────────────│       │
  │ id_registro (PK)            │       │
  │ fecha_sistema (PARTITION)   │       │
  │ codigo_insumo (CLUSTER)     │       │
  │ proveedor_pk (FK, CLUSTER)  │───────┘
  │ cantidad_ingresada          │
  │ precio_unitario             │
  │ importe_total               │
  │ precio_sin_iva              │
  │ almacen_destino             │
  └─────────────────────────────┘
       ▲              ▲
       │              │
  compras_limpio   pedidos.csv
  (222,201 rows)  (133,779 rows)

  ┌─────────────────────────────┐
  │ etl_anomaly_log             │
  │─────────────────────────────│
  │ source_file                 │
  │ row_number                  │
  │ anomaly_type                │
  │ action_taken                │
  └─────────────────────────────┘


  SISTEMA TRANSACCIONAL (Sheets + BigQuery):
  ┌─────────────────────────────┐
  │ FondoRevolventeLedger       │  ← LEDGER-001 (schema canónico)
  │ (BigQuery + SQLite + Excel) │
  └──────────┬──────────────────┘
             │
             │ folio_dsa + codigo_insumo
             │ se resuelven contra DW para:
             │   - Proyección de proveedores (STAT-001)
             │   - Validación de precios (CAT-001)
             │   - Cuadro comparativo (COMP-001)
             ▼
  ┌──────────────────────────────────────────────────────┐
  │ STAT-001: AffinityProjectionEngine                   │
  │   SQL: fact_pedidos_historicos + dim_proveedores     │
  │   Output: SupplierAffinityScore (enriquecido)        │
  ├──────────────────────────────────────────────────────┤
  │ CAT-001: BigQueryClient                              │
  │   SQL: fact_recepciones_historicas + dim_proveedores │
  │   Output: CacheService (historial real)              │
  ├──────────────────────────────────────────────────────┤
  │ COMP-001: NormativeValidator                         │
  │   Input: cotización PDF (Gemini)                     │
  │   Cross-ref: precios del DW como benchmark           │
  └──────────────────────────────────────────────────────┘


  Entidades DEPRECATED (post-patch v5.1):
  ✗ PurchaseHistory      → FactRecepcionesHistoricas + FactPedidosHistoricos
  ✗ LegacyPurchaseRecord → FondoRevolventeLedger / FactRecepcionesHistoricas
  ✗ ExcelRow             → FondoRevolventeLedger
  ✗ MatrixEntry          → EstudioMercadoLineas
  ✗ ComparativeMatrix    → EstudioMercadoMetadata
```

**FK Integrity Post-Patch:**

- `FactPedidosHistoricos.rfc_proveedor` → `DimProveedor.rfc_proveedor` (FK semántica)
- `FactRecepcionesHistoricas.proveedor_pk` → `DimProveedor.proveedor_pk` (FK semántica)
- Todos los FK transaccionales previos de v4.1 preservados e íntegros.
- No existen huérfanos referenciales.

**Volumetría Real Confirmada:**
| Tabla | Registros | Tamaño Estimado | Partición |
|---|---|---|---|
| `fact_recepciones_historicas` | 222,201 | ~111 MB | `fecha_sistema` |
| `fact_pedidos_historicos` | ~132,987 | ~53 MB | `fecha_pedido` |
| `dim_proveedores` | ~512 | ~0.1 MB | N/A |
| `etl_anomaly_log` | ~800+ | ~0.5 MB | N/A |
| **Total DW** | **~355,900** | **~165 MB** | Dentro de 10 GB Free Tier |

---

## 5. PARTE C — RESUMEN CONSOLIDADO DE CAMBIOS

### Estado de Módulos Post-Integración (v5.1)

| #   | ID           | Estado v5.0    | Estado v5.1        | Delta                    |
| --- | ------------ | -------------- | ------------------ | ------------------------ |
| 1   | LEDGER-001   | UNCHANGED      | UNCHANGED          | —                        |
| 2   | SCAN-001     | UNCHANGED      | UNCHANGED          | —                        |
| 3   | AI-001       | UNCHANGED      | UNCHANGED          | —                        |
| 4   | **SYNC-001** | UNCHANGED      | **PATCH_REVISION** | FIX-01 + INS-04 + FIX-08 |
| 5   | **EXP-001**  | UNCHANGED      | **PATCH_REVISION** | INS-03                   |
| 6   | CAT-001      | PATCH_REVISION | PATCH_REVISION     | —                        |
| 7   | MAIL-001     | UNCHANGED      | UNCHANGED          | —                        |
| 8   | **ETL-001**  | PATCH_REVISION | PATCH_REVISION     | FIX-04                   |
| 9   | PROXY-001    | UNCHANGED      | UNCHANGED          | —                        |
| 10  | **AUTH-001** | UNCHANGED      | **PATCH_REVISION** | FIX-03                   |
| 11  | **QUOT-001** | UNCHANGED      | **PATCH_REVISION** | FIX-05                   |
| 12  | STAT-001     | PATCH_REVISION | PATCH_REVISION     | —                        |
| 13  | INBOUND-001  | UNCHANGED      | UNCHANGED          | —                        |
| 14  | **COMP-001** | UNCHANGED      | **PATCH_REVISION** | FIX-02 + FIX-06 + FIX-07 |
| 15  | **DW-001**   | NEW_MODULE     | **PATCH_REVISION** | INS-01                   |
