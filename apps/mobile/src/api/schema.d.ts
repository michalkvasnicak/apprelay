// Generated from the Rust API. Do not edit; run pnpm api:generate.
export interface paths {
  "/health": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get: operations["health"];
    put?: never;
    post?: never;
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/apple/proxy": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get?: never;
    put?: never;
    post: operations["relay"];
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/devices": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get?: never;
    put?: never;
    post: operations["register_device"];
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/devices/{device_id}/pairing": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get?: never;
    put?: never;
    post: operations["start_pairing_attempt"];
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/devices/{device_id}/pairing-code": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get: operations["get"];
    put?: never;
    post?: never;
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/devices/{device_id}/push-token": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get?: never;
    put: operations["put"];
    post?: never;
    delete: operations["delete"];
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/pairing-attempts": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get?: never;
    put?: never;
    post: operations["create_pairing_attempt"];
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
  "/v1/pairing-attempts/{token}": {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    get: operations["get_pairing_attempt"];
    put?: never;
    post?: never;
    delete?: never;
    options?: never;
    head?: never;
    patch?: never;
    trace?: never;
  };
}
export type webhooks = Record<string, never>;
export interface components {
  schemas: {
    AttemptStatus:
      | {
          /** @enum {string} */
          status: "pending";
        }
      | {
          /** @enum {string} */
          status: "pairing";
        }
      | {
          /** @enum {string} */
          status: "success";
        }
      | {
          reason: components["schemas"]["FailureReason"];
          /** @enum {string} */
          status: "failed";
        }
      | {
          /** @enum {string} */
          status: "expired";
        };
    CreatedAttempt: {
      /** Format: date-time */
      expires_at: string;
      token: string;
    };
    ErrorResponse:
      | {
          /** @enum {string} */
          code: "invalid_request";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "app_key_required";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "device_token_required";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "service_unavailable";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "not_found";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "conflict";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "expired";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "rate_limited";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "upstream_failure";
          error: string;
        }
      | {
          /** @enum {string} */
          code: "upstream_timeout";
          error: string;
        };
    /** @enum {string} */
    FailureReason: "pairing_failed" | "timeout";
    Health: {
      status: string;
    };
    PairingCodeResponse: {
      code: string;
      /** Format: date-time */
      expires_at: string;
    };
    /** @enum {string} */
    ProxyMethod: "GET" | "POST";
    ProxyRequest: {
      /** @description Standard base64-encoded bytes. Omit for an empty request body. */
      body?: string;
      /** @description Only protocol headers are allowed; never include the AppRelay API key. */
      headers?: {
        [key: string]: string;
      };
      method: components["schemas"]["ProxyMethod"];
      /** @description HTTPS URL on an explicitly permitted Apple or anisette endpoint. */
      url: string;
    };
    ProxyResponse: {
      /** @description Standard base64-encoded upstream response bytes. */
      body: string;
      /** @description Only content-type, retry-after, and Apple routing info are returned. */
      headers: {
        [key: string]: string;
      };
      /**
       * Format: int32
       * @description Upstream HTTP status, including rejections and redirects (never followed).
       */
      status: number;
    };
    PushTokenRequest: {
      token: string;
    };
    RegistrationRequest: {
      wireguard_public_key: string;
    };
    RegistrationResponse: {
      address: string;
      /** Format: uuid */
      device_id: string;
      /** Format: date-time */
      expires_at: string;
      /** Format: int64 */
      lifetime_seconds: number;
      token: string;
      wireguard: components["schemas"]["WireguardConfiguration"];
    };
    StartRequest: {
      token: string;
    };
    WireguardConfiguration: {
      allowed_ips: string[];
      /** Format: int32 */
      persistent_keepalive_seconds: number;
      server_public_key: string;
    };
  };
  responses: never;
  parameters: never;
  requestBodies: never;
  headers: never;
  pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
  health: {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    requestBody?: never;
    responses: {
      200: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["Health"];
        };
      };
    };
  };
  relay: {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    requestBody: {
      content: {
        "application/json": components["schemas"]["ProxyRequest"];
      };
    };
    responses: {
      /** @description Upstream response; inspect the envelope status */
      200: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ProxyResponse"];
        };
      };
      /** @description Invalid proxy request or destination/header policy */
      400: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Request too large */
      413: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Expected application/json */
      415: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Invalid JSON fields */
      422: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Upstream TLS, DNS, transport, encoding or response-size failure */
      502: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Proxy concurrency limit reached */
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Proxy timed out */
      504: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  register_device: {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    requestBody: {
      content: {
        "application/json": components["schemas"]["RegistrationRequest"];
      };
    };
    responses: {
      /** @description Device registered */
      201: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["RegistrationResponse"];
        };
      };
      /** @description Invalid registration */
      400: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Invalid app key or device credential */
      401: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Request body too large */
      413: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Expected application/json */
      415: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Invalid JSON fields */
      422: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Rate limited; see Retry-After */
      429: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      /** @description Provisioning unavailable */
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  start_pairing_attempt: {
    parameters: {
      query?: never;
      header?: never;
      path: {
        device_id: string;
      };
      cookie?: never;
    };
    requestBody: {
      content: {
        "application/json": components["schemas"]["StartRequest"];
      };
    };
    responses: {
      /** @description Attempt claimed and durably queued; same-device retries are safe */
      201: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      400: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      401: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      404: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      409: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      410: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      413: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      415: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      422: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  get: {
    parameters: {
      query?: never;
      header?: never;
      path: {
        /** @description Registered device ID */
        device_id: string;
      };
      cookie?: never;
    };
    requestBody?: never;
    responses: {
      200: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["PairingCodeResponse"];
        };
      };
      /** @description Invalid device ID */
      400: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      401: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      404: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  put: {
    parameters: {
      query?: never;
      header?: never;
      path: {
        /** @description Registered device ID */
        device_id: string;
      };
      cookie?: never;
    };
    requestBody: {
      content: {
        "application/json": components["schemas"]["PushTokenRequest"];
      };
    };
    responses: {
      /** @description Push token saved */
      204: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      /** @description Invalid device ID or token */
      400: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      401: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      413: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      415: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      422: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  delete: {
    parameters: {
      query?: never;
      header?: never;
      path: {
        /** @description Registered device ID */
        device_id: string;
      };
      cookie?: never;
    };
    requestBody?: never;
    responses: {
      /** @description Push token removed */
      204: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      /** @description Invalid device ID */
      400: {
        headers: {
          [name: string]: unknown;
        };
        content?: never;
      };
      401: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  create_pairing_attempt: {
    parameters: {
      query?: never;
      header?: never;
      path?: never;
      cookie?: never;
    };
    requestBody?: never;
    responses: {
      201: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["CreatedAttempt"];
        };
      };
      429: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
  get_pairing_attempt: {
    parameters: {
      query?: never;
      header?: never;
      path: {
        /** @description Secret pairing attempt capability */
        token: string;
      };
      cookie?: never;
    };
    requestBody?: never;
    responses: {
      200: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["AttemptStatus"];
        };
      };
      404: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      429: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
      503: {
        headers: {
          [name: string]: unknown;
        };
        content: {
          "application/json": components["schemas"]["ErrorResponse"];
        };
      };
    };
  };
}
