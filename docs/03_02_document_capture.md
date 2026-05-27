# 3.2. [SCAN-001] MÓDULO: DOCUMENT_CAPTURE_PIPELINE

**ESTADO:** UNCHANGED

## 2.3.1 REQUERIMIENTOS FUNCIONALES

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

## 2.3.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `Document`
  - **Properties:** `{id: UUID, expedition_id: UUID, document_type: DocumentTypeEnum, file_name: String, drive_file_id: String, mime_type: String, blob_size_bytes: Int64, created_at: ISO8601}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`drive_file_id`, `document_type`, `expedition_id`)

- **ENUM:** `DocumentTypeEnum` = `[OFFICIO_SOLICITUD, NEGATIVA_EXISTENCIA]`

## 2.3.3 CONTRACTS & INTERFACES

- **COMPONENT:** `ScannerBridge` | **TRIGGER:** User action → WebSocket command `SCAN_START`
  - **DATA_CONTRACT (Input):** `{scanner_id: String, resolution_dpi: Int32, color_mode: Enum[COLOR, GRAYSCALE, BW], output_format: Enum[PDF, JPEG, PNG]}`
  - **DATA_CONTRACT (Output):** `{status: Enum[SUCCESS, DEVICE_BUSY, ERROR], blob: Base64, page_count: Int32, error_message: String?}`

- **COMPONENT:** `DriveUploader` | **TRIGGER:** Event `SCAN_COMPLETED`
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, document_type: DocumentTypeEnum, blob: Base64, mime_type: String}`
  - **DATA_CONTRACT (Output):** `{drive_file_id: String, web_view_link: String}`
