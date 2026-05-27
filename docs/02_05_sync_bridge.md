# 2.5. [SYNC-001] MÓDULO: EDGE_SYNCHRONIZATION_BRIDGE

**ESTADO:** PATCH_REVISION — Excel local muta de Data Mart de solo lectura a **Transactive Store** bidireccional de lectura/escritura actualizado por clave compuesta `(folio_dsa, codigo)`.

## 2.5.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-SYNC-01·P3] BidirectionalTransactiveStoreEnforcement:**
  - **Desc:** El archivo Excel local (.xlsx sobre SMB) opera como un **Transactive Store** modificable e interactivo de lectura y escritura. El daemon Rust actualiza el estatus de los trámites y el progreso de los hitos operativos a medida que ocurren las transiciones en la FSM cloud, manteniendo sincronizado el Edge de forma resiliente.
  - **Logic:** El daemon Rust realiza de forma segura el lock-check (`~$filename.xlsx`). Al detectar actualizaciones en SQLite, busca la fila correspondiente por `(folio_dsa, codigo)`. Si existe, sobrescribe los campos específicos correspondientes al hito de la FSM (Bloques 3, 4 y 5); si no existe, ejecuta un _append_ de la fila completa denormalizada.
  - **Post-Condition:** Excel local modificado y actualizado dinámicamente con cero corrupción en datos.

- **[ID-REQ-SYNC-01·P] SQLiteWALPersistence:**
  - **Desc:** El daemon Rust persiste en la base de datos SQLite local (`.db`) como Write-Ahead Log intermedio basado en la entidad canónica `FondoRevolventeLedger` antes de subir a BigQuery.
  - **Logic:** Si BigQuery es inaccesible, los registros permanecen en SQLite con `sync_status = PENDING` y se reintentan en el siguiente ciclo.
  - **Post-Condition:** Datos locales persistidos en SQLite en tránsito.

- **[ID-REQ-SYNC-01·P2] BigQueryBatchLoad:**
  - **Desc:** Reemplazar la escritura directa a Sheets por un job de carga batch (`WRITE_APPEND`) a BigQuery desde el agente Rust, consumiendo registros de SQLite.
  - **Post-Condition:** Filas disponibles en BigQuery; registros locales marcados como `SYNCED`.

- **[ID-REQ-SYNC-02] PessimisticFileLockDetection:**
  - **Desc:** Verificar ausencia del archivo de bloqueo temporal (`~$<NombreDelArchivo>.xlsx`) antes de realizar cualquier actualización o append en el Excel local. Si existe, aplicar Exponential Backoff y suspender la escritura.
  - **Post-Condition:** Fila insertada o actualizada en Excel local seguro sin corrupción.

- **[ID-REQ-SYNC-03] ExponentialBackoffRetry:**
  - **Desc:** Reintento exponencial con techo de 300 segundos para la I/O de Excel y persistencia del puntero para evitar re-procesamiento.
  - **Post-Condition:** Estado `SYNC_BLOCKED` si se exceden 10 reintentos.

- **[ID-REQ-SYNC-04] DriveToSMBFileSync:**
  - **Desc:** El daemon Rust replica archivos nuevos depositados en Google Drive local al directorio SMB correspondiente.
  - **Logic:** File watcher en Drive local → copia a `SMB_EXPEDIENTES/<folio_code>/`.
  - **Post-Condition:** Archivo disponible en ambas ubicaciones.

- **[ID-REQ-SYNC-05] LegacyCSVPersistence:**
  - **Desc:** Los CSV de xfarma se parsean en chunks y se transforman en registros `FondoRevolventeLedger` para ser insertados en SQLite antes de BigQuery.
  - **Post-Condition:** Datos legacy cargados transaccionalmente de manera atómica.

- **[ID-REQ-SYNC-06] TransactiveRowUpdateOrAppend:**
  - **Desc:** Escribir y actualizar filas en el Excel local según el `folio_dsa` y `codigo` del insumo, mapeando dinámicamente los campos actualizados de `FondoRevolventeLedger` (Bloques 3, 4 y 5) conforme progresan las fases operativas cloud, o realizando un full row append si no existe.
  - **Logic:** Mapea el struct completo denormalizado al layout de columnas en Excel. Busca por clave primaria compuesta `(folio_dsa, codigo)`. Ejecuta actualización in-situ si coincide, previniendo incoherencias transaccionales.

### 2.5.1.1 — Política de Control de Concurrencia Pesimista e Integridad de Red (Rust-to-SMB)

El archivo Microsoft Excel ubicado en la red local SMB opera como un **Transactive Store** bidireccional. Para evitar condiciones de carrera (_Data Race_) o corrupción por bloqueos concurrentes de usuarios humanos, el Agente en Rust implementa un subsistema de aislamiento mediante exclusión mutua basada en el sistema de archivos de Windows 11.

#### 2.5.1.1.1 Mecanismo Antibloqueo (_Starvation Prevention Protocol_)

**Paso 1 — Detección Atómica del Lock:**
Antes de iniciar cualquier escritura, el hilo de Rust busca la existencia del archivo descriptor de bloqueo oculto generado nativamente por Microsoft Excel:

```
IF EXISTS(~$<NombreDelArchivo>.xlsx)
  THEN lock_detected = true
  ELSE lock_detected = false
```

**Paso 2 — Estrategia ante Lock Activo (Usuario editando localmente):**

- El agente **no aborta** la operación.
- El agente **no genera un archivo duplicado** ("copia en conflicto"), manteniendo la integridad de la red SMB limpia.
- El agente suspende la escritura en el Excel y almacena la transacción pendiente en el caché de persistencia local seguro de **SQLite (`local_buffer.db`)** operando en modo _Write-Ahead Log (WAL)_.

**Paso 3 — Exponential Backoff:**

```
delay = min(base_delay * 2^retry_counter, max_delay_300s)
// base_delay = 2 segundos
// max_delay = 300 segundos (5 minutos)
// max_retries = 10
// Si retry_counter > max_retries → status = SYNC_BLOCKED
//   → Notificar a UI del operador
//   → Preservar last_processed_id en Sheets/BigQuery
```

**Paso 4 — Liberación y Vaciado (_Flushing_):**
Un hilo de vigilancia con estrategia de _Exponential Backoff_ testea el descriptor de bloqueo de Windows. En el instante en que el usuario humano cierra el Excel local y el archivo `~$` desaparece:

1. El daemon de Rust toma el control exclusivo del archivo mediante la API Win32 (`FileShare.None`).
2. Extrae en bloque (_Batch Read_) los registros acumulados en la SQLite local.
3. Ejecuta una operación de tipo UPDATE o APPEND de filas utilizando el crate `calamine`/`openpyxl-rs`.
4. Libera el manejador del archivo inmediatamente tras la escritura.
5. Actualiza `sync_status` de los registros en SQLite a `SYNCED`.

#### 2.5.1.1.2 Propiedades de Consistencia

| Propiedad | Garantía |
| :--- | :--- |
| **Consistencia eventual perfecta** | Los datos jamás se pierden: la nube (BigQuery) es la fuente de verdad; SQLite es el buffer resiliente. |
| **Cero corrupción de archivos** | Nunca se escribe en Excel mientras el lock `~$` exista. |
| **Cero pérdida ante caída de red** | SQLite WAL persiste localmente. Upload a BigQuery se reintenta indefinidamente. |
| **Tablas dinámicas intactas** | Las fórmulas y tablas dinámicas nativas de la `Hoja3` del Excel se recalculan limpiamente al cerrar y reabrir, sin intervención manual. |
| **No duplicación de archivos** | No se generan "copias en conflicto" ni archivos temporales en la red SMB. |

#### 2.5.1.1.3 Diagrama de Flujo de Decisión

```
                    ┌─────────────────┐
                    │ Registro        │
                    │ pendiente en    │
                    │ SQLite WAL      │
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │ ¿Existe lock    │
                    │ ¿~$archivo.xlsx?│
                    └────┬───────┬────┘
                         │       │
                    SÍ   │       │   NO
                         ▼       ▼
                ┌──────────┐  ┌──────────────┐
                │ Suspende │  │ Win32        │
                │ Escritura│  │ FileShare    │
                │          │  │ .None        │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
                ┌──────────┐  ┌──────────────┐
                │ Aplica   │  │ Batch Read   │
                │ Backoff  │  │ desde SQLite │
                │ (2^n seg)│  │              │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
                ┌──────────┐  ┌──────────────┐
                │ Re-test  │  │ UPDATE/      │
                │ Lock     │  │ APPEND en    │
                │          │  │ Excel        │
                └────┬─────┘  └──────┬───────┘
                     │               │
                     ▼               ▼
               ┌──────────┐   ┌──────────────┐
               │ ¿Libre?  │   │ Libera       │
               │ SÍ → Loop│   │ Handle       │
               │ NO → Wait│   │ sync_status  │
               └──────────┘   │ = SYNCED     │
                              └──────────────┘
```

## 2.5.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `SyncPointer`
  - **NOTA:** Entidad parcialmente deprecada. La función de `last_processed_row_id` es subsumida por `FondoRevolventeLedger.sync_status` en SQLite. Se mantiene para compatibilidad con el polling de Sheets (Google Sheets como control plane).
  - **Properties:** `{id: UUID, sheet_id: String, last_processed_row_id: Int64, last_processed_expedition_id: UUID, retry_count: Int32, status: SyncStatusEnum, updated_at: ISO8601}`
  - **Constraints:** PK(`id`), UNIQUE(`sheet_id`), NOT NULL(`last_processed_row_id`)

- **ENTITY:** `SQLiteWAL`
  - **Properties:** `{record_id: UUID, table_name: String, payload: JSON, status: WALStatusEnum, created_at: ISO8601, synced_at: ISO8601?}`
  - **Constraints:** PK(`record_id`), NOT NULL(`table_name`, `payload`, `status`)

- **ENTITY:** `BigQueryLoadJob`
  - **Properties:** `{job_id: String, dataset_id: String, table_id: String, row_count: Int64, status: BQJobStatusEnum, created_at: ISO8601, completed_at: ISO8601?}`
  - **Constraints:** PK(`job_id`), NOT NULL(`status`)

- **ENUM:** `SyncStatusEnum` = `[IN_SYNC, PENDING_RETRY, SYNC_BLOCKED, UP_TO_DATE]`
- **ENUM:** `WALStatusEnum` = `[PENDING, UPLOADING, SYNCED, FAILED]`
- **ENUM:** `BQJobStatusEnum` = `[RUNNING, DONE, FAILED]`

- **Excel Column Layout (v4.1 canonical Transactive Store):**
  ```
  | A: folio_dsa | B: tipo_tramite | C: fecha_recepcion | D: servicio_solicitante |
  | E: oficio_solicitud | F: codigo | G: descripcion | H: cantidad_solicitada |
  | I: unidad_medida | J: partida_especifica | K: usuario_asignado |
  | L: fecha_inicio_cotizacion | M: estatus_tramite | N: observaciones |
  | O: folio_supre | P: fecha_supre | Q: paquete_envio_caa |
  | R: fecha_recibido_caa | S: fecha_autorizacion_caa | T: folio_autorizacion_caa |
  | U: precio_unitario | V: monto_subtotal | W: monto_iva | X: monto_total_con_iva |
  | Y: cantidad_pedido | Z: numero_pedido | AA: fecha_pedido | AB: proveedor_rfc |
  | AC: estatus_entrega | AD: fecha_entrega_almacen | AE: numero_factura |
  | AF: fecha_factura | AG: fecha_envio_xml_rf | AH: fecha_pago |
  | AI: fecha_complemento_pago_rf |
  ```

## 2.5.3 CONTRACTS & INTERFACES

- **COMPONENT:** `SheetsPoller` | **TRIGGER:** Cron/Rust tokio interval
  - **DATA_CONTRACT (Input):** `{sheet_id: String, range: String, last_processed_row_id: Int64}`
  - **DATA_CONTRACT (Output):** `{new_rows: Array<FondoRevolventeLedger>, has_more: Boolean}`

- **COMPONENT:** `ExcelWriter` | **TRIGGER:** Queue drained + lock released
  - **DATA_CONTRACT (Input):** `{file_path: String, records: Array<FondoRevolventeLedger>, timeout: Duration}`
  - **DATA_CONTRACT (Output):** `{written: Int32, lock_detected: Boolean, new_pointer: Int64}`
  - **INVARIANT:** Realiza búsquedas llave `(folio_dsa, codigo)` para ejecutar UPDATE local en celdas, o inserta mediante APPEND si no existe registro previo.

- **COMPONENT:** `FilesystemWatcher` | **TRIGGER:** filesystem notification on sync folder
  - **DATA_CONTRACT (Input):** `{watch_path: String, event_type: Enum[CREATED, MODIFIED]}`
  - **DATA_CONTRACT (Output):** `{file_id: String, file_name: String, target_smb_path: String}`

- **COMPONENT:** `SQLiteManager` | **TRIGGER:** CSV file detected OR queued writes from any module
  - **DATA_CONTRACT (Input):** `{table_name: String, rows: Array<JSON>, transaction_mode: Enum[IMMEDIATE, DEFERRED]}`
  - **DATA_CONTRACT (Output):** `{inserted: Int32, transaction_committed: Boolean}`

- **COMPONENT:** `BigQueryBatchLoader` | **TRIGGER:** Cron interval OR threshold reached
  - **DATA_CONTRACT (Input):** `{project_id: String, dataset_id: String, table_id: String, pending_records: Array<SQLiteWAL>}`
  - **DATA_CONTRACT (Output):** `{job_id: String, rows_loaded: Int64, errors: Array<String>?}`
