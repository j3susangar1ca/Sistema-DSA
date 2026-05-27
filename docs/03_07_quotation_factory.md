# 3.7. [QUOT-001] MÓDULO: QUOTATION_DOCUMENT_FACTORY

**ESTADO:** PATCH_REVISION — Definición de plantilla legal obligatoria de correo y despacho dinámico de cotizaciones.

## 3.7.1 REQUERIMIENTOS FUNCIONALES

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

## 3.7.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `QuotationAssignment`
  - **Properties:** `{id: UUID, expedition_id: UUID, assigned_at: ISO8601, assigned_by: String, supplier_count: Int32, status: AssignmentStatusEnum, resolution_mode: ResolutionModeEnum, direct_supplier_count: Int32, affinity_supplier_count: Int32}`

- **ENTITY:** `QuotationDocument`
  - **Properties:** `{id: UUID, expedition_id: UUID, supplier_id: String, supplier_rfc: String, tracking_id: String(16), template_id: String, pdf_drive_id: String, generated_at: ISO8601, generated_by: String}`
  - **Constraints:** PK(`id`), UNIQUE(`tracking_id`)

- **ENTITY:** `QuotationDispatch`
  - **Properties:** `{id: UUID, quotation_doc_id: UUID, expedition_id: UUID, supplier_email: String, subject: String, gmail_message_id: String?, dispatched_at: ISO8601, dispatched_by: String}`

- **ENUM:** `AssignmentStatusEnum` = `[PENDING, ASSIGNED, DISPATCHED, PARTIALLY_RECEIVED, COMPLETED, EXPIRED]`
- **ENUM:** `ResolutionModeEnum` = `[DIRECT, HYBRID]`

## 3.7.3 CONTRACTS & INTERFACES

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
