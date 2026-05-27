# 5. [DW-001] MÓDULO: HISTORICAL_DATA_WAREHOUSE

**ESTADO:** PATCH_REVISION — Especificación de esquemas dimensionales extendidos de alta volumetría.

**Propósito:** Definir el modelo dimensional (esquema estrella) en Google BigQuery que consolidará los 355,980 registros históricos reales extraídos del sistema legacy xfarma/Dedalus del Hospital Civil de Guadalajara, siendo la **fuente de verdad analítica** consumida por los módulos STAT-001 y CAT-001.

## 3.10.1 REQUERIMIENTOS FUNCIONALES

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

## 3.10.2 PERSISTENCIA Y DATA MODEL

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

### 3.10.2.1 — Esquemas de Datos de Infraestructura de Alta Volumetría

Para dar soporte a la ingesta masiva del histórico institucional (~355,980 registros consolidados de xfarma y Dedalus), se implementa un modelo analítico columnar particionado en Google BigQuery (Cloud) acoplado a un motor in-memory transaccional en el Edge (SQLite via Rust).

#### 3.10.2.1.1 Tabla A: `fact_recepciones_historicas` (Carga masiva desde `compras_limpio.csv`)

- **Engine:** Google BigQuery (Columnar Store)
- **Partitioning:** `PARTITION BY fecha_sistema` (Mensual)
- **Clustering:** `CLUSTER BY codigo_insumo, proveedor_pk`
- **Volumetría:** 222,201 registros \| ~111 MB estimado

| Nombre del Campo | Tipo de Dato | Restricción / Indexación | Descripción |
| :--- | :--- | :--- | :--- |
| `id_registro` | `INT64` | PRIMARY KEY | Identificador único secuencial de la transacción. |
| `fecha_sistema` | `DATE` | REQUIRED, **PARTITION KEY** | Timestamp de confirmación en el Kernel del ERP (`mov_fecha_sys`). |
| `fecha_albaran` | `DATE` | NULLABLE | Fecha física del documento de entrega del proveedor (`mov_fecha_alb`). |
| `ejercicio_fiscal` | `INT64` | REQUIRED | Año de afectación contable (`mov_ejercicio`). |
| `codigo_insumo` | `STRING(10)` | REQUIRED, **CLUSTER KEY** | Identificador numérico LPAD-normalizado (`fk_codigo`). |
| `partida_conac` | `STRING(4)` | REQUIRED | Derivado: `SUBSTR(codigo_insumo, 1, 4)`. Clasificador CONAC Nivel 4. |
| `descripcion` | `STRING` | REQUIRED | Texto técnico del bien o servicio (`descripcion`). |
| `cantidad_ingresada` | `NUMERIC(15,4)` | REQUIRED | Cantidad física volumétrica recibida en almacén (`mov_cantidad`). |
| `precio_unitario` | `NUMERIC(15,4)` | REQUIRED | Costo unitario antes de impuestos (`mov_precio_lin`). |
| `importe_total` | `NUMERIC(15,4)` | REQUIRED | Costo neto de la línea (`mov_impor_lin`). |
| `precio_sin_iva` | `NUMERIC(15,4)` | NULLABLE | Precio sin IVA extraído de `siniva` (limpio en `compras_limpio.csv`). |
| `proveedor_pk` | `INT64` | REQUIRED, **CLUSTER KEY**, FK semántica → `dim_proveedores` | Identificador foráneo del catálogo legacy. |
| `almacen_destino` | `STRING` | REQUIRED | Denominación del almacén receptor (ej. `ALMACEN GENERAL FAA`). |
| `created_at` | `TIMESTAMP` | REQUIRED | Metadato de auditoría de carga. |

**Invariantes:**

- `fecha_sistema` < `1990-01-01` → `NULL` + entrada en `etl_anomaly_log` (tipo: `DATE_IMPOSSIBLE`).
- `precio_sin_iva` > `importe_total` → registro descartado + anomaly log (tipo: `NUMERIC_OVERFLOW`).
- `cantidad_ingresada` < 0 → registro descartado + anomaly log.

#### 3.10.2.1.2 Tabla B: `fact_pedidos_historicos` (Carga masiva desde `pedidos.csv`)

- **Engine:** Google BigQuery (Columnar Store)
- **Partitioning:** `PARTITION BY fecha_pedido` (Mensual)
- **Clustering:** `CLUSTER BY codigo_insumo, rfc_proveedor`
- **Volumetría:** ~132,987 registros (133,779 - 792 RFC nulos) \| ~53 MB estimado

| Nombre del Campo | Tipo de Dato | Restricción / Indexación | Descripción |
| :--- | :--- | :--- | :--- |
| `numero_pedido` | `STRING` | PRIMARY KEY | Código del contrato o pedido asignado (`nro_pedido` cast a STRING). |
| `fecha_pedido` | `DATE` | REQUIRED, **PARTITION KEY** | Fecha de formalización legal (`fecha`, parse `DD/MM/YYYY`). |
| `rfc_proveedor` | `STRING` | REQUIRED, **CLUSTER KEY** | Registro Federal de Contribuyentes (`nif`, TRIM). Si nulo → **ROW EXCLUDED**. |
| `razon_social_proveedor` | `STRING` | REQUIRED | Denominación comercial del licitante (`proveedor`, TRIM). |
| `codigo_insumo` | `STRING(10)` | REQUIRED, **CLUSTER KEY** | Clave del catálogo institucional (`codigo`, LPAD 10). |
| `descripcion` | `STRING` | REQUIRED | Texto técnico del bien o servicio (`articulo`). |
| `cantidad_pedida` | `NUMERIC(15,4)` | REQUIRED | Volumen total comprometido (`cantidad`). |
| `precio_con_iva` | `NUMERIC(15,4)` | REQUIRED | Costo unitario pactado con IVA (`precio`). |
| `precio_sin_iva` | `NUMERIC(15,4)` | REQUIRED | Costo unitario pactado sin IVA (`precio_sin_iva`). |
| `created_at` | `TIMESTAMP` | REQUIRED | Metadato de auditoría de carga. |

**Columnas EXCLUIDAS del CSV (100% nulas en los 133,779 registros):**
`atributo_portal`, `familia_terap`, `subfam_terap`, `grupo_terap`, `principio_act`, `grupo`, `subgrupo`, `familia`, `subfamilia` (9 columnas eliminadas del schema de carga, reduciendo ancho de 18 a 9 campos útiles + 1 metadato).

#### 3.10.2.1.3 Tabla C: `estudio_mercado_lineas` (Cuadro Comparativo Normalizado 3NF)

- **Engine:** Google Sheets (Cloud Interface) + BigQuery (replica analítica)
- **PK Compuesta:** `(folio_dsa, proveedor_rfc)`

| Nombre del Campo | Tipo de Dato | Restricción / Indexación | Descripción |
| :--- | :--- | :--- | :--- |
| `folio_dsa` | `STRING` | PK, FK → `Expedition.folio_code` | Identificador del expediente raíz. |
| `proveedor_rfc` | `STRING` | PK | RFC de la empresa evaluada. |
| `proveedor_padron_id` | `STRING` | NULLABLE | Registro vigente en el padrón de proveedores HCG (ej. `P21221`). |
| `proveedor_razon_social` | `STRING` | REQUIRED | Denominación comercial completa. |
| `tiempo_entrega_dias` | `INT64` | NULLABLE | Plazo ofertado de suministro. |
| `tipo_dias` | `STRING` | ENUM: `["NATURALES", "HABILES"]` | Naturaleza del plazo. |
| `condiciones_pago` | `STRING` | REQUIRED | Términos comerciales (ej. "30 días crédito"). |
| `correo_contacto` | `STRING` | NULLABLE | Email del representante comercial. |
| `precio_unitario_ofertado` | `NUMERIC(15,4)` | REQUIRED | Valor unitario de la oferta económica sin IVA. |
| `importe_total_ofertado` | `NUMERIC(15,4)` | REQUIRED | Valor total de la oferta económica. |
| `moneda` | `STRING(3)` | REQUIRED, DEFAULT `'MXN'` | Código ISO 4217 de la moneda. |
| `cumple_anexo_tecnico` | `BOOLEAN` | REQUIRED | Bandera de idoneidad determinada por Gemini y/o validación humana. |
| `estatus_validacion` | `STRING` | ENUM: `["VALIDADO", "DEFICIENTE_NORMATIVAMENTE", "PENDING_VALIDATION"]` | Resultado de la validación normativa. |
| `motivo_rechazo_normativo` | `STRING` | NULLABLE | Bitácora de descalificación (ej. "Vigencia menor a 30 días"). |
| `quotation_response_id` | `UUID` | FK → `QuotationResponse.id` | Enlace al PDF de la cotización original interceptada. |
| `gemini_raw_response` | `JSON` | NULLABLE | Payload completo retornado por Gemini para auditoría. |
| `created_at` | `TIMESTAMP` | REQUIRED | Metadato de auditoría. |

**Invariantes:**

- `estatus_validacion == "VALIDADO"` requiere `cumple_anexo_tecnico == true` AND `precio_unitario_ofertado > 0`.
- Consolidación automática al contar `COUNT(*) WHERE estatus_validacion = 'VALIDADO' >= 3`.
- Una vez consolidada (`is_locked = true`), ninguna escritura programática es permitida en las celdas de datos.

## 3.10.3 CONTRACTS & INTERFACES

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
