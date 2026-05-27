# 4.5. [PROXY-001] MÓDULO: INTRANET_SCRAPING_PROXY

**ESTADO:** UNCHANGED

## 3.5.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-PROXY-01] ReverseSignalChannelSetup:**
  - **Desc:** Polling periódico asíncrono desde el Rust Agent sobre la tabla control sin puertos entrantes.
  - **Logic:** `SELECT * FROM scraping_requests WHERE status = 'PENDING' ORDER BY requested_at LIMIT 10`.

- **[ID-REQ-PROXY-02] IntranetHTTPRequest:**
  - **Desc:** Solicitud HTTP con `reqwest` y timeout de 10s contra la intranet.

- **[ID-REQ-PROXY-03] HTMLSemanticParsing:**
  - **Desc:** Parseo del DOM con crate `scraper` buscando clase CSS `.estatus-contrato`.

- **[ID-REQ-PROXY-04] ResponseChannelWriteback:**
  - **Desc:** Guardado de la respuesta inyectando estatus en `contract_status` y snapshot en `raw_html_snapshot`.

## 3.5.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `ScrapingRequest`
  - **Properties:** `{id: UUID, expedition_id: UUID, item_code: String(10), status: ScrapingStatusEnum, contract_status: ContractStatusEnum?, raw_html_snapshot: String?, retry_count: Int32, error_message: String?, requested_at: ISO8601, responded_at: ISO8601?}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), NOT NULL(`item_code`, `status`, `requested_at`)

- **ENUM:** `ScrapingStatusEnum` = `[PENDING, IN_PROGRESS, COMPLETED, FAILED, RETRYING]`
- **ENUM:** `ContractStatusEnum` = `[VIGENTE, EN_PROCESO, SIN_CONTRATO, DESCONOCIDO]`

## 3.5.3 CONTRACTS & INTERFACES

- **COMPONENT:** `ScrapingRequestWriter` | **TRIGGER:** FSM transition in Apps Script
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, item_code: String(10)}`
  - **DATA_CONTRACT (Output):** `{request_id: UUID, status: "PENDING"}`
