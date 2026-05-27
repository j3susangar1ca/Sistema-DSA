# 2.4. [AI-001] MÓDULO: MULTIMODAL_INFERENCE_ENGINE

**ESTADO:** UNCHANGED

## 2.4.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-AI-01] UnifiedContextInference / ResponseSchemaV2:**
  - **Desc:** Enviar ambos documentos escaneados simultáneamente a Gemini 1.5 Flash como un solo _Contexto Unificado_ para extracción de entidades clave y metadatos, usando una estructura anidada con dos dominios semánticos separados: `datos_solicitud` y `auditoria_cumplimiento` en `snake_case`.
  - **Logic:** Request multipart con ambos PDFs + prompt de instrucciones. responseSchema:
    ```json
    {
      "type": "object",
      "properties": {
        "datos_solicitud": {
          "type": "object",
          "properties": {
            "folio_dsa": { "type": "string" },
            "codigo_insumo": { "type": "string" },
            "descripcion": { "type": "string" },
            "unidad_medida": { "type": "string" }
          },
          "required": ["folio_dsa", "codigo_insumo", "descripcion"]
        },
        "auditoria_cumplimiento": {
          "type": "object",
          "properties": {
            "coincidencia_bienes_servicios": { "type": "boolean" },
            "coincidencia_cronologica_fechas": { "type": "boolean" },
            "analisis_correlacion": { "type": "string" }
          },
          "required": [
            "coincidencia_bienes_servicios",
            "coincidencia_cronologica_fechas",
            "analisis_correlacion"
          ]
        }
      },
      "required": ["datos_solicitud", "auditoria_cumplimiento"]
    }
    ```
  - **Post-Condition:** JSON almacenado en `ValidationResult.gemini_raw_response`. Los campos `items_match` y `dates_consistent` se derivan de la respuesta.

- **[ID-REQ-AI-02] CrossDocumentItemValidation:**
  - **Desc:** Validar determinísticamente que los bienes/servicios coincidan semánticamente, mapeando el resultado desde `auditoria_cumplimiento.coincidencia_bienes_servicios`.
  - **Logic:** Si es `false`, la FSM transiciona a `REJECTED_VALIDATION_FAILED`.
  - **Post-Condition:** Bandera booleana persistida en `ValidationResult.items_match`.

- **[ID-REQ-AI-03] TemporalConsistencyCheck:**
  - **Desc:** Verificar coherencia cronológica de fechas de emisión, mapeando el resultado desde `auditoria_cumplimiento.coincidencia_cronologica_fechas`.
  - **Post-Condition:** Bandera persistida en `ValidationResult.dates_consistent`.

## 2.4.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `ValidationResult`
  - **Properties:** `{id: UUID, expedition_id: UUID, items_match: Boolean, dates_consistent: Boolean, temporal_delta_days: Int32, gemini_raw_response: JSON, discrepancies: Array<Discrepancy>, validated_at: ISO8601, correlation_analysis: String, extracted_folio_dsa: String, extracted_item_code: String, extracted_description: String, extracted_unit_of_measure: String?}`
  - **Constraints:** PK(`id`), FK(`expedition_id` → `Expedition.id`), UNIQUE(`expedition_id`). `correlation_analysis` es NOT NULL.
  - **Propiedades derivadas actualizadas:**
    - `items_match` ← `auditoria_cumplimiento.coincidencia_bienes_servicios`
    - `dates_consistent` ← `auditoria_cumplimiento.coincidencia_cronologica_fechas`
    - `correlation_analysis` ← `auditoria_cumplimiento.analisis_correlacion`
    - `extracted_folio_dsa` ← `datos_solicitud.folio_dsa`
    - `extracted_item_code` ← `datos_solicitud.codigo_insumo`
    - `extracted_description` ← `datos_solicitud.descripcion`
    - `extracted_unit_of_measure` ← `datos_solicitud.unidad_medida` (nullable)

- **EMBEDDED_TYPE:** `Discrepancy`
  - **Properties:** `{field: String, doc1_value: String, doc2_value: String, severity: Enum[WARNING, BLOCKING]}`

## 2.4.3 CONTRACTS & INTERFACES

- **COMPONENT:** `GeminiInferenceClient` | **TRIGGER:** Event `DRIVE_UPLOADED`
  - **DATA_CONTRACT (Input):**
    ```json
    {
      "contents": [
        {
          "role": "user",
          "parts": [
            {
              "inlineData": {
                "mimeType": "application/pdf",
                "data": "<BASE64_OFICIO>"
              }
            },
            {
              "inlineData": {
                "mimeType": "application/pdf",
                "data": "<BASE64_NEGATIVA>"
              }
            },
            { "text": "<SYSTEM_PROMPT_WITH_VALIDATION_RULES>" }
          ]
        }
      ],
      "generationConfig": {
        "responseMimeType": "application/json",
        "responseSchema": {
          "type": "object",
          "properties": {
            "datos_solicitud": {
              "type": "object",
              "properties": {
                "folio_dsa": { "type": "string" },
                "codigo_insumo": { "type": "string" },
                "descripcion": { "type": "string" },
                "unidad_medida": { "type": "string" }
              },
              "required": ["folio_dsa", "codigo_insumo", "descripcion"]
            },
            "auditoria_cumplimiento": {
              "type": "object",
              "properties": {
                "coincidencia_bienes_servicios": { "type": "boolean" },
                "coincidencia_cronologica_fechas": { "type": "boolean" },
                "analisis_correlacion": { "type": "string" }
              },
              "required": [
                "coincidencia_bienes_servicios",
                "coincidencia_cronologica_fechas",
                "analisis_correlacion"
              ]
            }
          },
          "required": ["datos_solicitud", "auditoria_cumplimiento"]
        }
      }
    }
    ```
  - **DATA_CONTRACT (Output):** JSON conforme al `responseSchema` anterior.
