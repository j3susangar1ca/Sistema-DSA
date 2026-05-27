# 3.1. [EXP-001] MÓDULO: EXPEDITION_STATE_MACHINE

**ESTADO:** PATCH_REVISION — Definición de matriz extendida FSM y componentes validadores institucionales de control.

## 3.1.1 REQUERIMIENTOS FUNCIONALES

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

### 3.1.1.1 — Matriz de Transiciones Extendida con Componente Validador

La tabla FSM existente documenta los estados y transiciones. La siguiente matriz extendida añade el **componente validador responsable** y la **acción del sistema** para cada transición, proporcionando la especificación completa de implementación.

```
[ INGESTADO ] ──► [ CATALOGO_VALIDADO ] ──► [ EN_COTIZACION ] ──► [ EVALUADO_GEMINI ]
                                                                       │
[ AUT_CAA ] ◄── [ RECURSOS_FINANCIEROS ] ◄── [ COMPARATIVO_CONSOLIDADO ] ◄┘
     │
     ▼
[ COMPLETED ]
```

| Estado Origen | Evento Detonante | Estado Destino | Componente Validador | Acción del Sistema |
| :--- | :--- | :--- | :--- | :--- |
| `void` | Carga de Oficio / Escaneo | `INITIATED` → `SCANNING` | Frontend App (Apps Script) + SCAN-001 | Inicializa expediente. Asigna `folio_dsa`. Dispara `ScannerBridge`. |
| `SCANNING` | `DRIVE_UPLOADED` (x2 docs) | `DOCS_CAPTURED` | SCAN-001 + Drive API | Valida que ambos documentos (Oficio + Negativa) estén en Drive. |
| `DOCS_CAPTURED` | `INFERENCE_STARTED` | `INFERENCE_PENDING` | AI-001 (Gemini 1.5 Flash) | Envía ambos PDFs como Contexto Unificado. Extrae entidades y valida cruzadamente. |
| `INFERENCE_PENDING` | `VALIDATION_PASSED` | `VALIDATED` | AI-001 + ValidationResult | Si `items_match == true` AND `dates_consistent == true` → avanza. Si no → `REJECTED_VALIDATION_FAILED`. |
| `VALIDATED` | `CATALOG_VALID` | `CATALOG_CHECKED` | CAT-001 + BigQuery Cache | Búsqueda $O(1)$ contra caché. Verifica código activo. Sugiere proveedores históricos. |
| `CATALOG_CHECKED` | `EMAIL_SENT` | `PENDING_PROCUREMENT_VERIFICATION` | MAIL-001 + Gmail API | Genera tracking token SHA256. Envía correo a Coordinación de Adquisiciones. |
| `PENDING_PROCUREMENT_VERIFICATION` | `RESPONSE_RECEIVED` | `PROCEDENCIA_APROBADA` | MAIL-001 (polling Gmail 15 min) | Intercepta respuesta. Convierte hilo a PDF. Parsea semánticamente (APPROVED/DENIED). |
| `PROCEDENCIA_APROBADA` | `SUPPLIERS_ASSIGNED` | `ASIGNACION_PROVEEDORES` | QUOT-001 + STAT-001 | Consulta BigQuery para proveedores directos. Si < 3, fallback a afinidad CONAC. |
| `ASIGNACION_PROVEEDORES` | `QUOTATIONS_DISPATCHED` | `ESPERA_COTIZACIONES` | QUOT-001 (DocumentFactory + Gmail) | Genera PDF por proveedor (clon + merge + SHA256 hash). Despacho DIRECT o DRAFT. |
| `ESPERA_COTIZACIONES` | `QUOTATION_RECEIVED` | _(log-only, sin cambio)_ | INBOUND-001 (Gmail polling 10 min) | Registra evento individual. Extrae PDF. Invoca validación Gemini. Actualiza matriz. |
| `ESPERA_COTIZACIONES` | `COMPARATIVE_MATRIX_CONSOLIDATED` | `CUADRO_COMPARATIVO_CONSOLIDADO` | COMP-001 + Apps Script Engine | Al contar ≥ 3 cotizaciones validadas: congela hoja Sheets, calcula menor precio. |
| `ESPERA_COTIZACIONES` | `QUOTATION_DEADLINE_EXPIRED` | `COTIZACIONES_VENCIDAS` | EXP-001 (Time trigger 5 días hábiles) | Marca vencimiento. Permite cierre parcial. |
| `CUADRO_COMPARATIVO_CONSOLIDADO` | `AWARD_RECOMMENDATION_GENERATED` | `ADJUDICACION_SUGERIDA` | COMP-001 (AwardCalculator) | Genera tarjeta de recomendación. Presenta al operador. |
| `ADJUDICACION_SUGERIDA` | `AWARD_CONFIRMED` | `ENVIADO_RECURSOS_FINANCIEROS` | Operador Humano + EXP-001 | Confirmación explícita. Exporta PDF + XLSX. Deposita en Drive/SMB. Dispara hito SUPRE. |
| `ENVIADO_RECURSOS_FINANCIEROS` | `AUTORIZACION_SUBDIRECCION_RECEIVED` | `AUTORIZADO_SUBDIRECCION` | Operador Humano + EXP-001 | Registro de folio_supre y fecha_supre en Bloque 3 del Ledger. |
| `AUTORIZADO_SUBDIRECCION` | `USER_COMMIT` | `COMPLETED` | Operador Humano + EXP-001 | Commit final. Persiste fila en Sheets/BigQuery/Excel. Puebla Bloques 4 y 5. |
| Cualquier activo | `VALIDATION_FAILED` | `REJECTED_VALIDATION_FAILED` | AI-001 | Bloqueo irreversible del expediente. |
| Cualquier activo | `CATALOG_INVALID` | `REJECTED_CATALOG_INACTIVE` | CAT-001 | Bloqueo irreversible. |
| `PENDING_PROCUREMENT_VERIFICATION` | `PROCEDENCIA_DENEGADA` | `REJECTED_PROCUREMENT_DENIED` | MAIL-001 | Bloqueo irreversible. |

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

## 3.1.2 PERSISTENCIA Y DATA MODEL

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

## 3.1.3 CONTRACTS & INTERFACES

- **COMPONENT:** `FSMEngine` | **TRIGGER:** Any domain event
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, current_status: ExpeditionStatusEnum, triggering_event: EventTypeEnum, operator_email: String, payload: JSON?}`
  - **DATA_CONTRACT (Output):** `{transitioned: Boolean, new_status: ExpeditionStatusEnum, deadline_status: DeadlineStatusEnum?, validation_errors: Array<String>?}`

- **COMPONENT:** `MilestoneValidator` | **TRIGGER:** Mutation attempt on Bloque 3, 4, 5 fields
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, current_status: ExpeditionStatusEnum, target_block: Int32, fields: Map<String, Any>}`
  - **DATA_CONTRACT (Output):** `{allowed: Boolean, violations: Array<String>?}`
