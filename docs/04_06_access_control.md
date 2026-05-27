# 4.6. [AUTH-001] MÓDULO: ZERO_PASSWORD_ACCESS_CONTROL

**ESTADO:** PATCH_REVISION — Integración de logs de acceso asíncronos y Access Gate federado.

## 3.6.1 REQUERIMIENTOS FUNCIONALES

- **[ID-REQ-AUTH-01] FederatedIdentityInterception:**
  - **Desc:** Capturar de forma transparente `Session.getActiveUser().getEmail()` inyectando `operator_email`.

- **[ID-REQ-AUTH-02] WhitelistCacheValidation:**
  - **Desc:** Validar email contra la whitelist autorizada en cache con TTL de 6h.

- **[ID-REQ-AUTH-03] SessionContextInjection:**
  - **Desc:** Inyectar el `operator_email` en las propiedades de ejecución de Apps Script.

- **[ID-REQ-AUTH-04] AuditLogging:**
  - **Desc:** Registrar cada intento de acceso (exitoso o denegado) en la entidad `AccessAuditLog` con timestamp, email, resultado e IP del cliente (si disponible desde `request`).
  - **Logic:** Operación asíncrona, no bloqueante para el flujo principal.
    ```sql
    INSERT INTO AccessAuditLog(id, email, result, client_ip, timestamp)
    VALUES (UUID(), email, result, clientIp, NOW())
    ```
  - **Post-Condition:** Fila insertada. Intentos `DENIED_NOT_WHITELISTED` generan alerta en hoja `Control_Acceso` para revisión del administrador.

- **[ID-REQ-AUTH-05] ReferenceImplementation:**
  - **Desc:** Preservar la implementación de referencia del Access Gate para garantizar reproducibilidad.
  - **Logic:**

    ```javascript
    function evaluarPermisosAcceso() {
      const emailUsuario = Session.getActiveUser().getEmail();

      if (!emailUsuario) {
        registrarAcceso(emailUsuario, "DENIED_NO_IDENTITY");
        throw new Error("Acceso Denegado: Identidad no verificable.");
      }

      const cache = CacheService.getScriptCache();
      let listaBlanca = JSON.parse(cache.get("usuarios_autorizados"));

      if (!listaBlanca) {
        const sheet =
          SpreadsheetApp.openById(MASTER_SHEET_ID).getSheetByName(
            "Control_Acceso",
          );
        listaBlanca = sheet
          .getRange(2, 1, sheet.getLastRow() - 1, 1)
          .getValues()
          .flat();
        cache.put("usuarios_autorizados", JSON.stringify(listaBlanca), 21600);
      }

      if (listaBlanca.indexOf(emailUsuario) === -1) {
        registrarAcceso(emailUsuario, "DENIED_NOT_WHITELISTED");
        return { autorizado: false, email: emailUsuario };
      }

      registrarAcceso(emailUsuario, "GRANTED");
      return { autorizado: true, email: emailUsuario };
    }
    ```

  - **Post-Condition:** `AccessAuditLog` poblado en cada invocación.

## 3.6.2 PERSISTENCIA Y DATA MODEL

- **ENTITY:** `AccessControlEntry`
  - **Properties:** `{email: String, full_name: String, role: UserRoleEnum, is_active: Boolean, added_at: ISO8601}`
  - **Constraints:** PK(`email`), NOT NULL(`full_name`, `is_active`)

- **ENTITY:** `AccessAuditLog`
  - **Properties:** `{id: UUID, email: String, result: AccessResultEnum, client_ip: String?, timestamp: ISO8601}`
  - **Constraints:** PK(`id`), INDEX(`email`, `timestamp`)

- **ENUM:** `UserRoleEnum` = `[OPERADOR, SUPERVISOR, ADMIN]`
- **ENUM:** `AccessResultEnum` = `[GRANTED, DENIED_NOT_WHITELISTED, DENIED_NO_IDENTITY]`
