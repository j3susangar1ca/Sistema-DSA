# 1. ESPECIFICACIÓN DE REQUISITOS DE SOFTWARE Y ARQUITECTURA DE SISTEMAS (SRS/SAD)

## 1.1 SISTEMA DE ADJUDICACIÓN DE COMPRAS POR FONDO REVOLVENTE (SISTEMA-DSA)

---

### 1.1.1 METADATOS DEL DOCUMENTO

- **Código de Proyecto:** HCG-DSA-2026
- **Fecha de Emisión:** 2026-05-27
- **Cumplimiento Normativo:**
  - **ISO/IEC/IEEE 29148:2018** (Requirements Engineering)
  - **ISO/IEC 25010:2011** (Software Quality Model: Reliability, Performance Efficiency, Maintainability, Portability)
- **Autor:** Lead Systems Architect
- **Clasificación:** Confidencial / Técnico

---

### 1.1.2 RESUMEN EJECUTIVO Y METODOLOGÍA

Este documento constituye la especificación de requisitos formal y la descripción de arquitectura de software para el **Sistema-DSA**, una plataforma híbrida diseñada para el Hospital Civil de Guadalajara (HCG) que automatiza, valida y audita el proceso de adjudicación de compras a través del esquema de fondo revolvente.

El diseño sigue una metodología orientada a microservicios e integraciones híbridas, combinando componentes Cloud (Google Apps Script, BigQuery, Gemini AI, Gmail API, Google Drive API) con un agente local Edge programado en Rust que interactúa directamente con bases de datos transaccionales en SQLite y hojas de cálculo locales de Microsoft Excel actuando como un _Transactive Store_ en la red local (SMB).

#### Alineación con ISO/IEC 25010:

1. **Fiabilidad [REL]:** Implementación de un mecanismo transaccional local con SQLite WAL, detección preventiva de anomalías de fechas en datos legacy (`fecha < 1990` → NULL) y exclusión de registros huérfanos de proveedores.
2. **Eficiencia de Rendimiento [PER]:** Estructuración de un Data Warehouse estrella en BigQuery con particionamiento por fecha y clustering de insumos, logrando tiempos de consulta interactiva sub-segundo con procesamiento de cuotas gratuitas (Always Free Tier).
3. **Mantenibilidad [MNT]:** Separación estricta de responsabilidades en 15 módulos cohesivos con interfaces fuertemente tipadas y contratos de datos inmutables en formato JSON/DDL, además de la adopción estricta de la convención de nombres `snake_case` para bases de datos y serializaciones.
4. **Portabilidad [PRT]:** Adaptabilidad del Edge Agent Rust para operar de manera autónoma en cualquier entorno de red local con acceso a sistemas de archivos SMB.

---

## 1.2 CONVENCIÓN DE NOMENCLATURA

**DECLARACIÓN:** A partir de esta declaración, **todas las entidades, propiedades, columnas de BigQuery, campos de SQLite y miembros de structs Rust** adoptan `snake_case` como convención universal. Los identificadores de módulos (`[ID-XXX]`) y los nombres de componentes conservan PascalCase por convención de arquitectura de software. Las enumeraciones en Rust conservan PascalCase por idioma (`EstatusTramite`); en SQL/JSON se serializan como `UPPER_SNAKE_CASE`.

---

## 1.3 ÍNDICE DE LA DOCUMENTACIÓN MODULAR

A continuación se detalla la estructura física secuencial de la documentación del Sistema-DSA:

### 1.3.1 Capa de Introducción y Convenciones
- **[01_overview.md](file:///home/jesuslangarica/Sistema-DSA/docs/01_overview.md):** Metadatos del documento, resumen ejecutivo, alineación ISO/IEC 25010 e índice modular general.

### 1.3.2 Capa de Red, Datos e Integración Local (Edge)
- **[02_01_cloud_edge_protocol.md](file:///home/jesuslangarica/Sistema-DSA/docs/02_01_cloud_edge_protocol.md):** Protocolo de interoperabilidad Cloud-Edge libre de puertos entrantes (`Command Message` y Toko Polling loop).
- **[02_02_canonical_ledger.md](file:///home/jesuslangarica/Sistema-DSA/docs/02_02_canonical_ledger.md):** Módulo `[LEDGER-001]` de declaración del schema canónico `FondoRevolventeLedger` (Rust/SQL/Excel).
- **[02_03_document_capture.md](file:///home/jesuslangarica/Sistema-DSA/docs/02_03_document_capture.md):** Módulo `[SCAN-001]` para el hardware HP ScanJet, WebSocket Loopback e ingesta dual.
- **[02_04_inference_engine.md](file:///home/jesuslangarica/Sistema-DSA/docs/02_04_inference_engine.md):** Módulo `[AI-001]` del motor de inferencia multimodal Gemini 1.5 Flash (`datos_solicitud` y `auditoria_cumplimiento`).
- **[02_05_sync_bridge.md](file:///home/jesuslangarica/Sistema-DSA/docs/02_05_sync_bridge.md):** Módulo `[SYNC-001]` de exclusión mutua local Win32 (locks `~$`), SQLite WAL y Transactive Store Excel.

### 1.3.3 Capa de Lógica de Negocio y Procesos (FSM)
- **[03_01_fsm_engine.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_01_fsm_engine.md):** Módulo `[EXP-001]` con la matriz extendida de estados FSM, plazos de vencimiento y reconstructor de timelines.
- **[03_02_catalog_cache.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_02_catalog_cache.md):** Módulo `[CAT-001]` del servicio de caché analítica del catálogo e historiales BigQuery.
- **[03_03_email_interception.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_03_email_interception.md):** Módulos `[MAIL-001]` y `[INBOUND-001]` de despacho Gmail con tracking SHA256 e intercepción de cotizaciones.
- **[03_04_legacy_etl.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_04_legacy_etl.md):** Módulo `[ETL-001]` de ingesta y normalización de CSVs legacy de xfarma (`compras_limpio` y `pedidos`).
- **[03_05_intranet_scraping.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_05_intranet_scraping.md):** Módulo `[PROXY-001]` de scraping de contratos mediante proxy asíncrono con Toko y Crate scraper.
- **[03_06_access_control.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_06_access_control.md):** Módulo `[AUTH-001]` de seguridad federada federada (Session email), caché de whitelist y bitácora de accesos.
- **[03_07_quotation_factory.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_07_quotation_factory.md):** Módulo `[QUOT-001]` de generación de cotizaciones con Google Docs API y despacho híbrido directo/borradores.
- **[03_08_supplier_affinity.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_08_supplier_affinity.md):** Módulo `[STAT-001]` para la proyección analítica de afinidad CONAC basada en volumen y recencia ($I_A$).
- **[03_09_comparative_matrix.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_09_comparative_matrix.md):** Módulo `[COMP-001]` del motor del cuadro comparativo en 3NF, validación Gemini de ofertas e inmutabilidad.
- **[03_10_data_warehouse.md](file:///home/jesuslangarica/Sistema-DSA/docs/03_10_data_warehouse.md):** Módulo `[DW-001]` del diseño estrella de BigQuery (hechos y dimensiones) e integridad referencial.

### 1.3.4 Historial y Trazabilidad
- **[04_change_log.md](file:///home/jesuslangarica/Sistema-DSA/docs/04_change_log.md):** Registro de cambios, actualizaciones, parches de bases de datos y control de versiones.
