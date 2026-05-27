# 2.2. [LEDGER-001] MÓDULO: FONDO_REVOLVENTE_LEDGER

**ESTADO:** UNCHANGED

**Propósito:** Definir el modelo de datos canónico que unifica la representación del expediente de compra por fondo revolvente a través de todas las capas del sistema (Rust Edge Agent → SQLite WAL → BigQuery Cloud → Excel Transactive Store). Este módulo no contiene lógica de negocio propia; es la **declaración formal del schema** consumido por todos los demás módulos.

## 2.2.1 REQUERIMIENTOS FUNCIONALES

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

    | Bloque | Campos | FSM Phase de Población | Módulo Responsable |
    | --- | --- | --- | --- |
    | 1. Ingesta | `folio_dsa` → `partida_especifica` | `INITIATED` → `DOCS_CAPTURED` | SCAN-001 + AI-001 |
    | 2. Control | `usuario_asignado` → `observaciones` | `INITIATED` → `COMPLETED` (siempre activo) | AUTH-001 + EXP-001 |
    | 3. SUPRE + CAA | `folio_supre` → `folio_autorizacion_caa` | `PENDING_PROCUREMENT_VERIFICATION` → `PROCEDENCIA_APROBADA` | EXP-001 + MAIL-001 |
    | 4. Pedido | `financieros` → `proveedor_rfc` | `ADJUDICACION_SUGERIDA` → `COMPLETED` | COMP-001 + QUOT-001 |
    | 5. Pasivo/Pago | `estatus_entrega` → `fecha_complemento_pago_rf` | `COMPLETED` (post-cierre) | SYNC-001 |

  - **Post-Condition:** Documento de referencia vivo que guía la implementación de cada módulo.

- **[ID-REQ-LEDGER-03] EstatusTramiteFSMBridge:**
  - **Desc:** Definir el mapeo bidireccional determinista entre los 6 valores de `EstatusTramite` (Rust/legacy) y los 20 valores de `ExpeditionStatusEnum` (FSM cloud), permitiendo traducción en ambas direcciones sin pérdida de semántica.
  - **Logic:** Tabla de mapeo:

    | EstatusTramite (Rust) | → ExpeditionStatusEnum (FSM) | Dirección |
    | --- | --- | --- |
    | `Cotizacion` | `ESPERA_COTIZACIONES`, `ASIGNACION_PROVEEDORES`, `CUADRO_COMPARATIVO_CONSOLIDADO`, `ADJUDICACION_SUGERIDA` | FSM → Rust: cualquiera de estos 4 estados se mapea a `Cotizacion` |
    | `RecursosFinancieros` | `ENVIADO_RECURSOS_FINANCIEROS` | FSM → Rust |
    | `AutorizadoCaa` | `PROCEDENCIA_APROBADA` | FSM → Rust |
    | `AutorizadoSub` | `AUTORIZADO_SUBDIRECCION` | FSM → Rust |
    | `Cancelado` | `REJECTED_VALIDATION_FAILED`, `REJECTED_CATALOG_INACTIVE`, `REJECTED_PROCUREMENT_DENIED`, `COTIZACIONES_VENCIDAS` | FSM → Rust: cualquiera de estos se mapea a `Cancelado` |
    | `Entregado` | `COMPLETED` | FSM → Rust |

  - **Post-Condition:** Función de traducción `fn fsm_to_rust(status: ExpeditionStatusEnum) -> EstatusTramite` implementable sin ambigüedad.

- **[ID-REQ-LEDGER-04] TypeSafetyEnforcement:**
  - **Desc:** Garantizar que todos los campos numéricos financieros usen `f64` en Rust y `NUMERIC`/`DECIMAL(18,4)` en SQL, que las fechas sean `NaiveDate` (sin timezone implícita) y que los campos opcionales usen `Option<T>` en lugar de valores centinela (`""`, `0`, `"N/A"`).
  - **Logic:** Validación en el pipeline de transformación: `csv_async` parse → `Result<T, ParseError>` → `None` para campos faltantes, nunca string vacío.
  - **Post-Condition:** Cero valores centinela en la base de datos.

## 2.2.2 PERSISTENCIA Y DATA MODEL

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

## 2.2.3 CONTRACTS & INTERFACES

- **COMPONENT:** `LedgerSerializer` | **TRIGGER:** Any write operation to SQLite or BigQuery
  - **DATA_CONTRACT (Input):** `FondoRevolventeLedger` struct (Rust)
  - **DATA_CONTRACT (Output):** Flat key-value map compatible with BigQuery `insertAll` or SQLite `INSERT`
  - **INVARIANT:** `FinancieroSnapshot` → se despliega en 4 columnas planas (`precio_unitario`, `monto_subtotal`, `monto_iva`, `monto_total_con_iva`). Si `financieros == None`, las 4 columnas se insertan como `NULL`.

- **COMPONENT:** `EstatusBridge` | **TRIGGER:** Sync operation (Rust → Excel Transactive Store)
  - **DATA_CONTRACT (Input):** `{fsm_status: ExpeditionStatusEnum}`
  - **DATA_CONTRACT (Output):** `{rust_status: EstatusTramite}`
  - **Logic:** Mapeo según tabla de [ID-REQ-LEDGER-03].
