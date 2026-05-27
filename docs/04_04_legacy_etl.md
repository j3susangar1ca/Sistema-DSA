# 4.4. [ETL-001] MÓDULO: LEGACY_CSV_INGESTION_PIPELINE

**ESTADO:** PATCH_REVISION — Column mapping real desde headers de xfarma para compras (13 cols) y pedidos (18 cols). Date anomaly detection y exclusión de RFCs nulos. LPAD de insumos.

## 3.4.1 REQUERIMIENTOS FUNCIONALES

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

## 3.4.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `IngestionJob`
  - **Properties:** `{id: UUID, source_file_path: String, total_rows: Int64, processed_rows: Int64, error_rows: Int64, status: IngestionJobStatusEnum, started_at: ISO8601, completed_at: ISO8601?, source_file_type: SourceFileTypeEnum, rows_skipped_null_rfc: Int64?, columns_excluded: Int32?, anomalies_detected: Int64}`

- **ENTITY:** `CSVParseError`
  - **Properties:** `{id: UUID, job_id: UUID, line_number: Int64, raw_value: String, error_message: String, created_at: ISO8601}`

- **ENUM:** `SourceFileTypeEnum` = `[COMPRAS_LIMPIO, PEDIDOS, COMPRAS_RAW, FONDO_REVOLVENTE_LEDGER]`
- **ENUM:** `IngestionJobStatusEnum` = `[QUEUED, PARSING, INSERTING, UPLOADING_BQ, COMPLETED, FAILED]`

### 3.4.2.1 COLUMN MAPPING TABLE — `compras_limpio.csv` → `fact_recepciones_historicas`

| Columna CSV | Tipo CSV | Campo Destino | Tipo BQ | Transformación |
| --- | --- | --- | --- | --- |
| `id_registro` | INT | `id_registro` | INT64 | Directo |
| `mov_fecha_sys` | `YYYY-MM-DD` | `fecha_sistema` | DATE | `PARSE_DATE`; si < `1990-01-01` → NULL + anomaly log |
| `mov_fecha_alb` | `YYYY-MM-DD` | `fecha_albaran` | DATE | `PARSE_DATE`; nullable |
| `mov_ejercicio` | INT | `ejercicio_fiscal` | INT64 | Directo |
| `fk_codigo` | STRING | `codigo_insumo` | STRING(10) | `LPAD(TRIM(), 10, '0')` |
| `descripcion` | STRING | `descripcion` | STRING | `TRIM()` |
| `mov_cantidad` | NUMERIC | `cantidad_ingresada` | NUMERIC | Cast; reject negatives |
| `mov_precio_lin` | NUMERIC | `precio_unitario` | NUMERIC | Cast |
| `mov_impor_lin` | NUMERIC | `importe_total` | NUMERIC | Cast |
| `siniva` | NUMERIC | `precio_sin_iva` | NUMERIC | Cast; validate < `importe_total` |
| `proveedor_pk` | INT | `proveedor_pk` | INT64 | Directo |
| `proveedor_nombre` | STRING | _(→ dim_proveedores)_ | — | Join para enriquecer dimensión |
| `almacen_deno` | STRING | `almacen_destino` | STRING | `TRIM()` |

### 3.4.2.2 COLUMN MAPPING TABLE — `pedidos.csv` → `fact_pedidos_historicos`

| Columna CSV | Tipo CSV | Campo Destino | Tipo BQ | Transformación |
| --- | --- | --- | --- | --- |
| `nro_pedido` | INT | `numero_pedido` | STRING | `CAST AS STRING` |
| `fecha` | `DD/MM/YYYY` | `fecha_pedido` | DATE | `PARSE_DATE('%d/%m/%Y')` |
| `proveedor` | STRING | `razon_social_proveedor` | STRING | `TRIM()` |
| `nif` | STRING | `rfc_proveedor` | STRING | `TRIM()`; si NULL/empty → **SKIP ROW** + anomaly log |
| `codigo` | STRING(10) | `codigo_insumo` | STRING(10) | `LPAD(TRIM(), 10, '0')` |
| `articulo` | STRING | `descripcion` | STRING | `TRIM()` |
| `cantidad` | NUMERIC | `cantidad_pedida` | NUMERIC | Cast; reject negatives |
| `precio` | NUMERIC | `precio_con_iva` | NUMERIC | Cast |
| `precio_sin_iva` | NUMERIC | `precio_sin_iva` | NUMERIC | Cast |
| `atributo_portal` | — | **EXCLUIDA** | — | 100% nula (133,779/133,779) |
| `familia_terap` | — | **EXCLUIDA** | — | 100% nula |
| `subfam_terap` | — | **EXCLUIDA** | — | 100% nula |
| `grupo_terap` | — | **EXCLUIDA** | — | 100% nula |
| `principio_act` | — | **EXCLUIDA** | — | 100% nula |
| `grupo` | — | **EXCLUIDA** | — | 100% nula |
| `subgrupo` | — | **EXCLUIDA** | — | 100% nula |
| `familia` | — | **EXCLUIDA** | — | 100% nula |
| `subfamilia` | — | **EXCLUIDA** | — | 100% nula |

## 3.4.3 CONTRACTS & INTERFACES

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
