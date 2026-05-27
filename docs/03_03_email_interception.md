# 3.3. [MAIL-001] / [INBOUND-001] MÓDULOS DE AUTOMATIZACIÓN E INTERCEPCIÓN DE EMAIL

Este archivo unifica las especificaciones para el despacho y seguimiento de correos salientes (`MAIL-001`) y la intercepción y procesamiento de cotizaciones entrantes de proveedores (`INBOUND-001`).

---

## 3.3.1 [MAIL-001] ASYNC_EMAIL_INTERCEPTION

**ESTADO:** UNCHANGED

### 3.3.1.1 REQUERIMIENTOS FUNCIONALES

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

### 3.3.1.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `EmailTracking`
  - **Properties:** `{id: UUID, expedition_id: UUID, tracking_token: String(16), subject: String, recipient_email: String, sent_at: ISO8601, responded_at: ISO8601, response_pdf_drive_id: String, parsed_decision: EmailDecisionEnum, status: EmailStatusEnum}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), UNIQUE(`tracking_token`)

- **ENTITY:** `Notification`
  - **Properties:** `{id: UUID, expedition_id: UUID, recipient_user_id: String, message: String, is_read: Boolean, created_at: ISO8601}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`)

- **ENUM:** `EmailStatusEnum` = `[SENT, AWAITING_RESPONSE, RESPONSE_RECEIVED, PARSED, TIMED_OUT]`
- **ENUM:** `EmailDecisionEnum` = `[APPROVED, DENIED, MANUAL_REVIEW_REQUIRED]`

### 3.3.1.3 CONTRACTS & INTERFACES

- **COMPONENT:** `EmailDispatcher` | **TRIGGER:** FSM transition to `CATALOG_CHECKED`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, folio_code: String, item_code: String, item_description: String, recipient_email: String, body_template: String}`
  - **DATA_CONTRACT (Output):** `{email_message_id: String, tracking_token: String(16), thread_id: String}`

- **COMPONENT:** `GmailPollingWorker` | **TRIGGER:** Apps Script Time-driven trigger (15 min)
  - **DATA_CONTRACT (Input):** `{search_query: String, known_tracking_tokens: Array<String>}`
  - **DATA_CONTRACT (Output):** `{matched_threads: Array<JSON>}`

---

## 3.3.2 [INBOUND-001] SUPPLIER_QUOTATION_INTERCEPTION

**ESTADO:** UNCHANGED

### 3.3.2.1 REQUERIMIENTOS FUNCIONALES

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

### 3.3.2.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `QuotationResponse`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_rfc: String, supplier_email: String, gmail_message_id: String, gmail_thread_id: String, subject: String, pdf_drive_id: String, pdf_file_name: String, received_at: ISO8601, processed_at: ISO8601, processed_by: String}`
  - **Constraints:** PK(`id`), UNIQUE(`gmail_message_id`), FK(`expedition_id` → `Expedition.id`)

### 3.3.2.3 CONTRACTS & INTERFACES

- **COMPONENT:** `SupplierResponsePoller`
  - **DATA_CONTRACT (Input):** `{search_query: String, known_dispatched_suppliers: Map<String, Array<String>>}`
  - **DATA_CONTRACT (Output):** `{matched_threads: Array<JSON>}`
