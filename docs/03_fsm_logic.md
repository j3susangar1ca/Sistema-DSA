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
