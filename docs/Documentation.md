<scratchpad_tecnico>
1. **EXISTING_STATE:** None. Primera ejecución — no existen IDs previos ni entidades declaradas.
2. **IDENTIFIED_CHANGES:** Descomposición completa del sistema en 6 módulos funcionales: (a) Captura de documentos vía escáner, (b) Inferencia multimodal con Gemini, (c) Puente de sincronización Cloud-to-Edge con CQRS, (d) Motor de expedientes con FSM y Event Sourcing, (e) Servicio de caché de catálogo institucional, (f) Interceptación asíncrona de correo electrónico.
3. **ISO_25010_IMPACT:**
   - **Seguridad [SEC]:** Zero Inbound Ports (RNF-004), Tracking ID criptográfico en correos, bloqueo pesimista de archivos para evitar corrupción.
   - **Fiabilidad [REL]:** Exponential Backoff en reintento de escritura, tolerancia a bloqueo de I/O por mutex de Excel, consistencia eventual garantizada por el puntero de última fila procesada.
   - **Mantenibilidad [MNT]:** Event Sourcing (append-only log) permite auditoría completa y reconstrucción de estado. CQRS desacopla escritura cloud de persistencia local. FSM explícita con estados finitos documentados.
4. **MAPPING:**
   - Operador de Escaneo → SCAN-001 → Document
   - Motor IA → AI-001 → ValidationResult
   - Rust Daemon → SYNC-001 → ExcelRow, ExpeditionEvent
   - Administrador de Trámites → EXP-001 → Expedition
   - Sistema de UI → CAT-001 → CatalogItem
   - Coordinación de Adquisiciones → MAIL-001 → EmailTracking
</scratchpad_tecnico>

---

### [SCAN-001] MÓDULO: DOCUMENT_CAPTURE_PIPELINE
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-SCAN-01] ScannerHardwareBridge:**
    *   **Desc:** Iniciar secuencia de escaneo desde el navegador web hacia el HP ScanJet 3000 s mediante Scanner.js (Asprise) sobre loopback local, sin intervención de software intermedio por parte del usuario.
    *   **Logic:** Frontend establece conexión WebSocket/Loopback → envía comando `SCAN_START` → driver TWAIN/WIA captura imagen → stream de bytes (PDF o Base64) retorna al Frontend.
    *   **Post-Condition:** Blob binario disponible en memoria del cliente; emite evento `SCAN_COMPLETED`.

*   **[ID-REQ-SCAN-02] DualDocumentIngestion:**
    *   **Desc:** Capturar obligatoriamente dos documentos por expediente: (1) Oficio de Solicitud y (2) Negativa de Existencia, como precondición para activar el pipeline de inferencia.
    *   **Logic:** `IF documentCount < 2 THEN status = SCANNING; ELSE status = DOCS_CAPTURED AND enable AI_PIPELINE`.
    *   **Post-Condition:** Ambos blobs binarios almacenados en Google Drive en la carpeta del expediente; emite evento `DRIVE_UPLOADED`.

*   **[ID-REQ-SCAN-03] DriveUploadWithIndexing:**
    *   **Desc:** Cargar cada documento escaneado a Google Drive en una carpeta indexada por `folioCode` del expediente, generando un `driveFileId` persistente.
    *   **Logic:** `DriveApp.getFolderById(PARENT_FOLDER).createFolder(folioCode).createFile(blob)`. Retorna `driveFileId` para almacenamiento en entidad `Document`.
    *   **Post-Condition:** Archivo accesible vía API; `Document.driveFileId` poblado.

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `Document`
    *   **Properties:** `{id: UUID, expeditionId: UUID, documentType: DocumentTypeEnum, fileName: String, driveFileId: String, mimeType: String, blobSizeBytes: Int64, createdAt: ISO8601}`
    *   **Constraints:** PK(`id`), FK(`expeditionId` → `Expedition.id`), NOT NULL(`driveFileId`, `documentType`, `expeditionId`)

*   **ENUM:** `DocumentTypeEnum` = `[OFFICIO_SOLICITUD, NEGATIVA_EXISTENCIA]`

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `ScannerBridge` | **TRIGGER:** User action → WebSocket command `SCAN_START`
    *   **DATA_CONTRACT (Input):** `{scannerId: String, resolutionDPI: Int32, colorMode: Enum[COLOR, GRAYSCALE, BW], outputFormat: Enum[PDF, JPEG, PNG]}`
    *   **DATA_CONTRACT (Output):** `{status: Enum[SUCCESS, DEVICE_BUSY, ERROR], blob: Base64, pageCount: Int32, errorMessage: String?}`

*   **COMPONENT:** `DriveUploader` | **TRIGGER:** Event `SCAN_COMPLETED`
    *   **DATA_CONTRACT (Input):** `{expeditionId: UUID, documentType: DocumentTypeEnum, blob: Base64, mimeType: String}`
    *   **DATA_CONTRACT (Output):** `{driveFileId: String, webViewLink: String}`

---

### [AI-001] MÓDULO: MULTIMODAL_INFERENCE_ENGINE
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-AI-01] UnifiedContextInference:**
    *   **Desc:** Enviar ambos documentos escaneados (Oficio de Solicitud + Negativa de Existencia) simultáneamente a Gemini 1.5 Flash como un solo *Contexto Unificado* para extracción de entidades clave y metadatos.
    *   **Logic:** Construir `multipart` request con ambos blobs PDF + prompt de instrucciones. Forzar `responseSchema` estricto (JSON mode) para eliminar variabilidad en la salida.
    *   **Post-Condition:** Objeto JSON estructurado con entidades extraídas almacenado en `ValidationResult.geminiRawResponse`.

*   **[ID-REQ-AI-02] CrossDocumentItemValidation:**
    *   **Desc:** Validar determinísticamente que los bienes/servicios solicitados en el Oficio de Solicitud coincidan semánticamente con los declarados en la Negativa de Existencia.
    *   **Logic:** Gemini retorna `itemsMatch: Boolean` con `discrepancies: Array<{field: String, doc1Value: String, doc2Value: String}>`. Si `itemsMatch == false`, FSM → `REJECTED_VALIDATION_FAILED`.
    *   **Post-Condition:** Bandera booleana persistida en `ValidationResult.itemsMatch`.

*   **[ID-REQ-AI-03] TemporalConsistencyCheck:**
    *   **Desc:** Verificar que las fechas de emisión de ambos documentos mantengan coherencia cronológica y que el delta temporal sea razonable respecto a la fecha del sistema.
    *   **Logic:** `IF dateOficio > dateNegativa OR deltaDays > MAX_THRESHOLD THEN datesConsistent = false`. Gemini calcula el delta y lo retorna como `temporalDeltaDays: Int32`.
    *   **Post-Condition:** Bandera persistida en `ValidationResult.datesConsistent`.

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `ValidationResult`
    *   **Properties:** `{id: UUID, expeditionId: UUID, itemsMatch: Boolean, datesConsistent: Boolean, temporalDeltaDays: Int32, geminiRawResponse: JSON, discrepancies: Array<Discrepancy>, validatedAt: ISO8601}`
    *   **Constraints:** PK(`id`), FK(`expeditionId` → `Expedition.id`), UNIQUE(`expeditionId`) — una validación por expediente.

*   **EMBEDDED_TYPE:** `Discrepancy`
    *   **Properties:** `{field: String, doc1Value: String, doc2Value: String, severity: Enum[WARNING, BLOCKING]}`

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `GeminiInferenceClient` | **TRIGGER:** Event `DRIVE_UPLOADED` (ambos documentos disponibles)
    *   **DATA_CONTRACT (Input):**
        ```json
        {
          "contents": [
            { "role": "user", "parts": [
              { "inlineData": { "mimeType": "application/pdf", "data": "<BASE64_OFICIO>" } },
              { "inlineData": { "mimeType": "application/pdf", "data": "<BASE64_NEGATIVA>" } },
              { "text": "<SYSTEM_PROMPT_WITH_VALIDATION_RULES>" }
            ]}
          ],
          "generationConfig": {
            "responseMimeType": "application/json",
            "responseSchema": {
              "type": "OBJECT",
              "properties": {
                "folioCode": { "type": "STRING" },
                "itemsMatch": { "type": "BOOLEAN" },
                "datesConsistent": { "type": "BOOLEAN" },
                "temporalDeltaDays": { "type": "INTEGER" },
                "extractedItems": { "type": "ARRAY", "items": { "type": "STRING" } },
                "discrepancies": { "type": "ARRAY", "items": { "type": "OBJECT" } }
              },
              "required": ["itemsMatch", "datesConsistent", "temporalDeltaDays"]
            }
          }
        }
        ```
    *   **DATA_CONTRACT (Output):** JSON conforme al `responseSchema` anterior.

---

### [SYNC-001] MÓDULO: EDGE_SYNCHRONIZATION_BRIDGE
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-SYNC-01] CloudToEdgePolling:**
    *   **Desc:** Ejecutar un sondeo periódico (intervalo configurable, recomendado 5 min) desde el daemon Rust contra la API de Google Sheets para detectar filas nuevas cuyo `lastProcessedId` sea inferior al último `expeditionId` registrado en la hoja maestra.
    *   **Logic:** `GET Sheets API → filter rows WHERE rowId > lastProcessedPointer → queue unprocessed rows for local persistence`.
    *   **Post-Condition:** Cola en memoria con filas pendientes de escritura local.

*   **[ID-REQ-SYNC-02] PessimisticFileLockDetection:**
    *   **Desc:** Antes de escribir en el archivo Excel local (.xlsx sobre SMB), verificar la ausencia del archivo temporal de bloqueo generado por Microsoft Excel (patrón `~$<NombreDelArchivo>.xlsx`).
    *   **Logic:** `IF EXISTS(~$<filename>.xlsx) THEN SUSPEND write, INCREMENT retryCounter, apply ExponentialBackoff(base=2s, max=300s), PERSIST lastProcessedId to Sheets`. `ELSE acquire handle → append rows → release handle immediately → UPDATE lastProcessedId`.
    *   **Post-Condition:** Filas escritas en Excel local SIN corrupción; puntero avanzado persistido en Sheets.

*   **[ID-REQ-SYNC-03] ExponentialBackoffRetry:**
    *   **Desc:** Implementar reintento exponencial para operaciones de escritura suspendidas por mutex de Excel, con techo máximo de 300 segundos y persistencia del puntero para evitar re-procesamiento.
    *   **Logic:** `delay = min(base * 2^retryCounter, 300)`. Si `retryCounter > MAX_RETRIES (10)`, emitir alerta y marcar como `SYNC_BLOCKED`.
    *   **Post-Condition:** Estado `SYNC_BLOCKED` notificado a UI o sistema de monitoreo.

*   **[ID-REQ-SYNC-04] DriveToSMBFileSync:**
    *   **Desc:** El daemon Rust detecta nuevos archivos depositados en las carpetas de expediente en Google Drive (vía `Drive for Desktop Daemon`) y los replica al directorio SMB local correspondiente.
    *   **Logic:** Filesystem watcher sobre la ruta de sincronización local de Google Drive. `ON_NEW_FILE → COPY to SMB_EXPEDIENTES/<folioCode>/`.
    *   **Post-Condition:** Archivo disponible en ambas ubicaciones (nube y local).

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `SyncPointer`
    *   **Properties:** `{id: UUID, sheetId: String, lastProcessedRowId: Int64, lastProcessedExpeditionId: UUID, retryCount: Int32, status: SyncStatusEnum, updatedAt: ISO8601}`
    *   **Constraints:** PK(`id`), UNIQUE(`sheetId`), NOT NULL(`lastProcessedRowId`)

*   **ENTITY:** `ExcelRow`
    *   **Properties:** `{rowId: Int64, expeditionId: UUID, folioCode: String, itemCode: String, itemDescription: String, quantity: Int32, amount: Decimal128, status: ExpeditionStatusEnum, createdAt: ISO8601}`
    *   **Constraints:** PK(`rowId`), FK(`expeditionId` → `Expedition.id`)

*   **ENUM:** `SyncStatusEnum` = `[IN_SYNC, PENDING_RETRY, SYNC_BLOCKED, UP_TO_DATE]`

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `SheetsPoller` | **TRIGGER:** Cron/Rust `tokio::time::interval` (configurable)
    *   **DATA_CONTRACT (Input):** `{sheetId: String, range: String, lastProcessedRowId: Int64}`
    *   **DATA_CONTRACT (Output):** `{newRows: Array<ExcelRow>, hasMore: Boolean}`

*   **COMPONENT:** `ExcelWriter` | **TRIGGER:** Queue drained of pending rows + file lock released
    *   **DATA_CONTRACT (Input):** `{filePath: String, rows: Array<ExcelRow>, timeout: Duration}`
    *   **DATA_CONTRACT (Output):** `{written: Int32, lockDetected: Boolean, newPointer: Int64}`

*   **COMPONENT:** `FilesystemWatcher` | **TRIGGER:** `inotify`/`ReadDirectoryChangesW` event on Drive sync folder
    *   **DATA_CONTRACT (Input):** `{watchPath: String, eventType: Enum[CREATED, MODIFIED]}`
    *   **DATA_CONTRACT (Output):** `{fileId: String, fileName: String, targetSMBPath: String}`

---

### [EXP-001] MÓDULO: EXPEDITION_STATE_MACHINE
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-EXP-01] EventSourcingAppendOnlyLog:**
    *   **Desc:** Registrar cada acción del sistema como un evento inmutable en una hoja dedicada de Google Sheets (`Events`), operando como *Write-Ahead Log* de solo inserción.
    *   **Logic:** `INSERT INTO Events(expeditionId, eventType, payload, actor, timestamp)`. Nunca se actualizan ni eliminan filas existentes.
    *   **Post-Condition:** Evento persistido con `eventId` (UUID) y `timestamp` (ISO8601).

*   **[ID-REQ-EXP-02] FiniteStateMachineTransitions:**
    *   **Desc:** Gobernar todas las transiciones de estado del expediente mediante una tabla de transiciones determinista. Cada cambio de estado emite un evento `STATE_TRANSITION`.
    *   **Logic:** Tabla FSM:
        | Estado Actual | Evento Disparador | Estado Siguiente |
        |---|---|---|
        | `INITIATED` | `SCAN_STARTED` | `SCANNING` |
        | `SCANNING` | `DRIVE_UPLOADED` (x2 docs) | `DOCS_CAPTURED` |
        | `DOCS_CAPTURED` | `INFERENCE_STARTED` | `INFERENCE_PENDING` |
        | `INFERENCE_PENDING` | `VALIDATION_PASSED` | `VALIDATED` |
        | `VALIDATED` | `CATALOG_VALID` | `CATALOG_CHECKED` |
        | `CATALOG_CHECKED` | `EMAIL_SENT` | `PENDING_PROCUREMENT_VERIFICATION` |
        | `PENDING_PROCUREMENT_VERIFICATION` | `RESPONSE_RECEIVED` | `PROCEDENCIA_APROBADA` |
        | `PROCEDENCIA_APROBADA` | `USER_COMMIT` | `COMPLETED` |
        | Cualquier estado activo | `VALIDATION_FAILED` | `REJECTED_VALIDATION_FAILED` |
        | Cualquier estado activo | `CATALOG_INVALID` | `REJECTED_CATALOG_INACTIVE` |
        | `PENDING_PROCUREMENT_VERIFICATION` | `PROCEDENCIA_DENEGADA` | `REJECTED_PROCUREMENT_DENIED` |
    *   **Post-Condition:** `Expedition.status` actualizado; evento `STATE_TRANSITION` insertado en log.

*   **[ID-REQ-EXP-03] TimelineReconstruction:**
    *   **Desc:** Reconstruir el estado actual y la línea de tiempo visual del expediente proyectando la secuencia cronológica de eventos desde el *Append-Only Log*.
    *   **Logic:** `SELECT * FROM Events WHERE expeditionId = X ORDER BY timestamp ASC` → transformar a `Array<TimelineEntry>` con estado calculado, actor, y payload renderizable.
    *   **Post-Condition:** UI renderiza timeline (estilo Asana/GitHub) con todos los eventos del expediente.

*   **[ID-REQ-EXP-04] UserCommitConfirmation:**
    *   **Desc:** Requerir confirmación explícita del operador antes de persistir la transacción final en Google Sheets (Cloud Master Ledger) y cerrar el expediente.
    *   **Logic:** `IF status == PROCEDENCIA_APROBADA AND user.confirms THEN INSERT_ROW(Sheets) AND emit USER_COMMIT`. La operación es idempotente: un doble-click no genera duplicados.
    *   **Post-Condition:** Fila persistida en Sheets; estado `COMPLETED`; expediente cerrado.

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `Expedition`
    *   **Properties:** `{id: UUID, folioCode: String, status: ExpeditionStatusEnum, createdBy: String, createdAt: ISO8601, updatedAt: ISO8601}`
    *   **Constraints:** PK(`id`), UNIQUE(`folioCode`), NOT NULL(`status`, `folioCode`)

*   **ENTITY:** `ExpeditionEvent`
    *   **Properties:** `{id: UUID, expeditionId: UUID, eventType: EventTypeEnum, actor: String, payload: JSON, timestamp: ISO8601}`
    *   **Constraints:** PK(`id`), FK(`expeditionId` → `Expedition.id`), NOT NULL(`eventType`, `timestamp`), INDEX(`expeditionId`, `timestamp`)

*   **ENUM:** `ExpeditionStatusEnum` = `[INITIATED, SCANNING, DOCS_CAPTURED, INFERENCE_PENDING, VALIDATED, CATALOG_CHECKED, PENDING_PROCUREMENT_VERIFICATION, PROCEDENCIA_APROBADA, REJECTED_VALIDATION_FAILED, REJECTED_CATALOG_INACTIVE, REJECTED_PROCUREMENT_DENIED, COMPLETED]`

*   **ENUM:** `EventTypeEnum` = `[CREATED, SCAN_STARTED, SCAN_COMPLETED, DRIVE_UPLOADED, INFERENCE_STARTED, INFERENCE_COMPLETED, VALIDATION_PASSED, VALIDATION_FAILED, CATALOG_VALID, CATALOG_INVALID, EMAIL_SENT, RESPONSE_RECEIVED, PROCEDENCIA_APROBADA, PROCEDENCIA_DENEGADA, USER_COMMIT, STATE_TRANSITION]`

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `FSMEngine` | **TRIGGER:** Any domain event
    *   **DATA_CONTRACT (Input):** `{expeditionId: UUID, currentStatus: ExpeditionStatusEnum, triggeringEvent: EventTypeEnum, payload: JSON?}`
    *   **DATA_CONTRACT (Output):** `{transitioned: Boolean, newStatus: ExpeditionStatusEnum, validationErrors: Array<String>?}`
    *   **INVARIANT:** Si el par `(currentStatus, triggeringEvent)` no existe en la tabla de transiciones, retorna `{transitioned: false, validationErrors: ["Invalid transition"]}` sin mutar estado.

*   **COMPONENT:** `TimelineRenderer` | **TRIGGER:** UI requests expedition detail
    *   **DATA_CONTRACT (Input):** `{expeditionId: UUID}`
    *   **DATA_CONTRACT (Output):** `{entries: Array<{timestamp: ISO8601, eventType: EventTypeEnum, actor: String, label: String, details: JSON}>, currentStatus: ExpeditionStatusEnum}`

---

### [CAT-001] MÓDULO: CATALOG_CACHE_SERVICE
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-CAT-01] CatalogCacheWarmup:**
    *   **Desc:** Cargar el catálogo institucional completo y el historial de compras desde Google Sheets a `CacheService` de Apps Script al inicio de la sesión, serializándolos como diccionarios JSON (Tablas Hash) con TTL de 6 horas.
    *   **Logic:** `CacheService.getScriptCache().put("CATALOG", JSON.stringify(catalogMap), 21600)`. Se mantiene un segundo caché `HISTORY` para el historial de compras. Invalidación automática por TTL.
    *   **Post-Condition:** Búsquedas posteriores resueltas en $O(1)$ sin llamadas a Sheets API.

*   **[ID-REQ-CAT-02] ItemCodeLookup:**
    *   **Desc:** Al ingresar un código de 10 dígitos (ej. `2541011002`), verificar su existencia y estado (activo/inactivo) contra el catálogo institucional en caché.
    *   **Logic:** `catalogMap[itemCode]` → Si no existe o `isActive == false`, FSM → `REJECTED_CATALOG_INACTIVE`. Si existe y activo, extraer `knownSuppliers` del historial.
    *   **Post-Condition:** Estado `CATALOG_CHECKED` o `REJECTED_CATALOG_INACTIVE`.

*   **[ID-REQ-CAT-03] HistoricalSupplierSuggestion:**
    *   **Desc:** Al validar un código de bien/servicio, cruzar el `itemCode` con la tabla de historial de compras para extraer un arreglo deduplicado de proveedores previos y presentarlos como *Sugerencias de Mercado* en la UI.
    *   **Logic:** `historyMap[itemCode].suppliers` → `Array.from(new Set(suppliers))` → renderizar en dropdown/select del frontend.
    *   **Post-Condition:** UI muestra lista de proveedores sugeridos.

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `CatalogItem`
    *   **Properties:** `{code: String(10), description: String, category: String, isActive: Boolean, unitOfMeasure: String}`
    *   **Constraints:** PK(`code`), NOT NULL(`description`, `isActive`)

*   **ENTITY:** `PurchaseHistory`
    *   **Properties:** `{id: UUID, itemCode: String(10), supplierName: String, lastPurchaseDate: ISO8601, lastUnitPrice: Decimal128, currency: String(3)}`
    *   **Constraints:** PK(`id`), FK(`itemCode` → `CatalogItem.code`), INDEX(`itemCode`)

*   **CACHE_SCHEMA:**
    ```
    CATALOG: Map<String(10), CatalogItem>    // TTL: 21600s (6h)
    HISTORY: Map<String(10), Array<PurchaseHistory>>  // TTL: 21600s (6h)
    ```

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `CatalogLookupService` | **TRIGGER:** UI input change on `itemCode` field (debounced 300ms)
    *   **DATA_CONTRACT (Input):** `{itemCode: String(10)}`
    *   **DATA_CONTRACT (Output):**
        ```json
        {
          "found": true,
          "isActive": true,
          "item": { "code": "2541011002", "description": "Lidocaína...", "category": "Medicamentos" },
          "suggestedSuppliers": [
            { "name": "Proveedor A", "lastPrice": 150.00, "lastDate": "2024-08-15" }
          ]
        }
        ```
    *   **ERROR_CONTRACT:** `{found: false}` o `{found: true, isActive: false}` → bloquea UI con motivo legible.

---

### [MAIL-001] MÓDULO: ASYNC_EMAIL_INTERCEPTION
**ESTADO:** NEW_MODULE

#### H4.1. REQUERIMIENTOS FUNCIONALES

*   **[ID-REQ-MAIL-01] TrackingIdInjection:**
    *   **Desc:** Al generar el correo electrónico de solicitud a la Coordinación de Adquisiciones, inyectar un identificador de seguimiento criptográfico (`trackingToken`) en los metadatos del correo y un folio legible en el asunto (patrón: `[Referencia: DSA-FR-<folioCode>-<itemCode>]`).
    *   **Logic:** Generar `trackingToken = SHA256(expeditionId + timestamp + salt)[0:16]`. Almacenar en entidad `EmailTracking`.
    *   **Post-Condition:** Correo enviado; entidad `EmailTracking` creada con `status: SENT`.

*   **[ID-REQ-MAIL-02] GmailPollingTrigger:**
    *   **Desc:** Ejecutar un *Time-Driven Trigger* en Apps Script cada 15 minutos para interrogar la API de Gmail en busca de respuestas no leídas que contengan el patrón de referencia en el asunto.
    *   **Logic:** `GmailApp.search('subject:"DSA-FR-" is:unread label:inbox')` → iterar resultados → extraer `folioCode` e `itemCode` del asunto → match contra `EmailTracking` pendientes.
    *   **Post-Condition:** Hilos de correo coincidentes identificados; eventos `RESPONSE_RECEIVED` emitidos.

*   **[ID-REQ-MAIL-03] ThreadPdfCapture:**
    *   **Desc:** Convertir la cadena completa de correos (solicitud + respuesta de la Coordinación) en un documento PDF oficial mediante la función nativa `Thread.getAs(MimeType.PDF)`.
    *   **Logic:** `thread.getAs(MimeType.PDF)` → blob binario resultante se deposita en la carpeta del expediente en Google Drive.
    *   **Post-Condition:** PDF de la cadena de correos en Drive; `EmailTracking.responsePdfDriveId` poblado.

*   **[ID-REQ-MAIL-04] ResponseSemanticParsing:**
    *   **Desc:** Leer el cuerpo del mensaje de respuesta de la Coordinación para determinar si la procedencia del fondo revolvente fue aprobada o denegada.
    *   **Logic:** Extraer `body = message.getPlainBody()`. Análisis de keywords/semántica: `IF contains(["procede", "aprobado", "autorizado"]) THEN APPROVED; ELSE IF contains(["no procede", "denegado"]) THEN DENIED`. Para ambigüedad, marcar `MANUAL_REVIEW_REQUIRED`.
    *   **Post-Condition:** FSM transiciona a `PROCEDENCIA_APROBADA` o `REJECTED_PROCUREMENT_DENIED`.

*   **[ID-REQ-MAIL-05] SilentUINotification:**
    *   **Desc:** Al completar la intercepción y procesamiento de la respuesta, depositar un evento de notificación silenciosa para que la UI del operador se actualice sin requerir polling activo del cliente.
    *   **Logic:** Almacenar notificación en hoja `Notifications` de Sheets con `read: false`. El frontend consulta `Notifications` al refrescar o mediante polling ligero (30s).
    *   **Post-Condition:** Operador ve indicador de expediente listo para continuar.

#### H4.2. PERSISTENCIA Y DATA MODEL

*   **ENTITY:** `EmailTracking`
    *   **Properties:** `{id: UUID, expeditionId: UUID, trackingToken: String(16), subject: String, recipientEmail: String, sentAt: ISO8601, respondedAt: ISO8601, responsePdfDriveId: String, parsedDecision: EmailDecisionEnum, status: EmailStatusEnum}`
    *   **Constraints:** PK(`id`), FK(`expeditionId` → `Expedition.id`), UNIQUE(`trackingToken`), NOT NULL(`trackingToken`, `status`, `sentAt`)

*   **ENTITY:** `Notification`
    *   **Properties:** `{id: UUID, expeditionId: UUID, recipientUserId: String, message: String, isRead: Boolean, createdAt: ISO8601}`
    *   **Constraints:** PK(`id`), FK(`expeditionId` → `Expedition.id`), INDEX(`recipientUserId`, `isRead`)

*   **ENUM:** `EmailStatusEnum` = `[SENT, AWAITING_RESPONSE, RESPONSE_RECEIVED, PARSED, TIMED_OUT]`

*   **ENUM:** `EmailDecisionEnum` = `[APPROVED, DENIED, MANUAL_REVIEW_REQUIRED]`

#### H4.3. CONTRACTS & INTERFACES

*   **COMPONENT:** `EmailDispatcher` | **TRIGGER:** FSM transition to `CATALOG_CHECKED`
    *   **DATA_CONTRACT (Input):** `{expeditionId: UUID, folioCode: String, itemCode: String, itemDescription: String, recipientEmail: String, bodyTemplate: String}`
    *   **DATA_CONTRACT (Output):** `{emailMessageId: String, trackingToken: String(16), threadId: String}`

*   **COMPONENT:** `GmailPollingWorker` | **TRIGGER:** Apps Script `Time-driven` trigger (every 15 min)
    *   **DATA_CONTRACT (Input):** `{searchQuery: String, knownTrackingTokens: Array<String(16)>}`
    *   **DATA_CONTRACT (Output):** `{matchedThreads: Array<{threadId: String, trackingToken: String, snippet: String, isUnread: Boolean}>}`

*   **COMPONENT:** `ResponseProcessor` | **TRIGGER:** Matched thread detected by `GmailPollingWorker`
    *   **DATA_CONTRACT (Input):** `{threadId: String, expeditionId: UUID}`
    *   **DATA_CONTRACT (Output):** `{decision: EmailDecisionEnum, pdfDriveId: String, responseBody: String, respondedAt: ISO8601}`

---

**[INTEGRITY CHECK]** Todos los FK apuntan a entidades declaradas dentro de esta descomposición: `Expedition.id` referenciado por `Document`, `ValidationResult`, `ExpeditionEvent`, `EmailTracking`, `Notification`, `ExcelRow`. `CatalogItem.code` referenciado por `PurchaseHistory.itemCode`. No existen huérfanos referenciales.

**[PENDIENTE]** Para generaciones sucesivas, cualquier `INPUT_PROCESO` adicional deberá evaluar si modifica módulos existentes (→`[PATCH]`) o introduce capacidades nuevas (→`[MODULE]`). Se requiere contexto completo de IDs declarados arriba para mantener integridad referencial.
