# 3.2. [CAT-001] MÓDULO: CATALOG_CACHE_SERVICE

**ESTADO:** PATCH_REVISION — Queries BigQuery en Apps Script actualizadas para consumir de las tablas reales dimensionales del Data Warehouse.

## 3.2.1 REQUERIMIENTOS FUNCIONALES

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

## 3.2.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `CatalogItem`
  - **Properties:** `{code: String(10), description: String, category: String, is_active: Boolean, unit_of_measure: String}`
  - **Constraints:** PK(`code`)
  - **Backing Store:** BigQuery table `hospital-civil-4562.inventario.catalogo_bienes`

- **ENTITY:** `PurchaseHistory` → **DEPRECATED** (subsumida por las consultas directas a hechos `fact_recepciones_historicas` y `fact_pedidos_historicos` en `[DW-001]`).

## 3.2.3 CONTRACTS & INTERFACES

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
