# ESPECIFICACIÓN DE REQUISITOS DE SOFTWARE Y ARQUITECTURA DE SISTEMAS (SRS/SAD)

## SISTEMA DE ADJUDICACIÓN DE COMPRAS POR FONDO REVOLVENTE (SISTEMA-DSA)

---

### METADATOS DEL DOCUMENTO

- **Código de Proyecto:** HCG-DSA-2026
- **Fecha de Emisión:** 2026-05-27
- **Cumplimiento Normativo:**
  - **ISO/IEC/IEEE 29148:2018** (Requirements Engineering)
  - **ISO/IEC 25010:2011** (Software Quality Model: Reliability, Performance Efficiency, Maintainability, Portability)
- **Autor:** Lead Systems Architect
- **Clasificación:** Confidencial / Técnico

---

### RESUMEN EJECUTIVO Y METODOLOGÍA

Este documento constituye la especificación de requisitos formal y la descripción de arquitectura de software para el **Sistema-DSA**, una plataforma híbrida diseñada para el Hospital Civil de Guadalajara (HCG) que automatiza, valida y audita el proceso de adjudicación de compras a través del esquema de fondo revolvente.

El diseño sigue una metodología orientada a microservicios e integraciones híbridas, combinando componentes Cloud (Google Apps Script, BigQuery, Gemini AI, Gmail API, Google Drive API) con un agente local Edge programado en Rust que interactúa directamente con bases de datos transaccionales en SQLite y hojas de cálculo locales de Microsoft Excel actuando como un _Transactive Store_ en la red local (SMB).

#### Alineación con ISO/IEC 25010:

1. **Fiabilidad [REL]:** Implementación de un mecanismo transaccional local con SQLite WAL, detección preventiva de anomalías de fechas en datos legacy (`fecha < 1990` → NULL) y exclusión de registros huérfanos de proveedores.
2. **Eficiencia de Rendimiento [PER]:** Estructuración de un Data Warehouse estrella en BigQuery con particionamiento por fecha y clustering de insumos, logrando tiempos de consulta interactiva sub-segundo con procesamiento de cuotas gratuitas (Always Free Tier).
3. **Mantenibilidad [MNT]:** Separación estricta de responsabilidades en 15 módulos cohesivos con interfaces fuertemente tipadas y contratos de datos inmutables en formato JSON/DDL, además de la adopción estricta de la convención de nombres `snake_case` para bases de datos y serializaciones.
4. **Portabilidad [PRT]:** Adaptabilidad del Edge Agent Rust para operar de manera autónoma en cualquier entorno de red local con acceso a sistemas de archivos SMB.

---

## CONVENCIÓN DE NOMENCLATURA

**DECLARACIÓN:** A partir de esta declaración, **todas las entidades, propiedades, columnas de BigQuery, campos de SQLite y miembros de structs Rust** adoptan `snake_case` como convención universal. Los identificadores de módulos (`[ID-XXX]`) y los nombres de componentes conservan PascalCase por convención de arquitectura de software. Las enumeraciones en Rust conservan PascalCase por idioma (`EstatusTramite`); en SQL/JSON se serializan como `UPPER_SNAKE_CASE`.
