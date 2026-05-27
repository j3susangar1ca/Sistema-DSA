# 4.8. [STAT-001] MÓDULO: SUPPLIER_AFFINITY_PROJECTION

**ESTADO:** PATCH_REVISION — SQL analítico reescrito para explotar el Data Warehouse dimensional de `[DW-001]` (`fact_pedidos_historicos` + `dim_proveedores`) con enriquecimiento de precios pagados reales desde `fact_recepciones_historicas`.

## 3.8.1 REQUERIMIENTOS FUNCIONALES

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

## 3.8.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `SupplierAffinityScore`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_rfc: String, supplier_razon_social: String, supplier_email: String, partida_conac: String(4), total_adjudicaciones: Int32, ultima_compra: ISO8601, affinity_index: Float64, calculated_at: ISO8601, ultimo_precio_real: Decimal128?, total_recepciones: Int64, ultima_recepcion: ISO8601?}`

## 3.8.3 CONTRACTS & INTERFACES

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
