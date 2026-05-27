# 3.9. [COMP-001] MÓDULO: COMPARATIVE_MATRIX_ENGINE

**ESTADO:** PATCH_REVISION — Normalización 3NF: `MatrixEntry` + `ComparativeMatrix` reemplazados por `EstudioMercadoMetadata` + `EstudioMercadoLineas`.

## 3.9.1 REQUERIMIENTOS FUNCIONALES

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

## 3.9.2 PERSISTENCIA Y DATA MODEL

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

## 3.9.3 CONTRACTS & INTERFACES

- **COMPONENT:** `NormativeValidator` | **TRIGGER:** `QuotationResponse` persisted
  - **DATA_CONTRACT (Input):** `{expedition_id: UUID, pdf_drive_id: String, supplier_rfc: String, original_request: JSON}`
  - **DATA_CONTRACT (Output):** JSON compatible con `EstudioMercadoLineas`.

- **COMPONENT:** `MatrixConsolidator` | **TRIGGER:** Valid `EstudioMercadoLineas` count $\ge 3$
  - **DATA_CONTRACT (Input):** `{folio_dsa: String, threshold: Int32, operator_email: String}`
  - **DATA_CONTRACT (Output):** `{consolidated: Boolean, valid_count: Int32, is_locked: Boolean}`

- **COMPONENT:** `AwardCalculator` | **TRIGGER:** `COMPARATIVE_MATRIX_CONSOLIDATED` event
  - **DATA_CONTRACT (Input):** `{folio_dsa: String}`
  - **DATA_CONTRACT (Output):** JSON de estructura `AwardRecommendation`.
