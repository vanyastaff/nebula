/**
 * Credential protocol schemas with form field definitions.
 *
 * Each schema defines:
 * - The credential kind (maps to Rust protocol types)
 * - Display name and description for UI
 * - Form field definitions with validation rules
 *
 * Mirrors the protocol definitions from crates/credential/src/protocols/
 */

import type { CredentialKind, CredentialProtocolSchema } from "../domain/types";

/**
 * API Key protocol schema
 *
 * Maps to ApiKeyProtocol from crates/credential/src/protocols/api_key.rs
 * Used for services authenticated via bearer token / API key
 */
export const API_KEY_SCHEMA: CredentialProtocolSchema = {
  kind: "api_key",
  displayName: "API Key",
  description: "Authentication using a server URL and API token",
  icon: "key",
  fields: [
    {
      name: "server",
      label: "Server URL",
      type: "url",
      required: true,
      placeholder: "https://api.example.com",
      helpText: "Base URL of the service (e.g. https://api.github.com)",
      sensitive: false,
      validation: {
        pattern: "^https?://.*",
      },
    },
    {
      name: "token",
      label: "API Token",
      type: "password",
      required: true,
      placeholder: "",
      helpText: "Secret API token or personal access token",
      sensitive: true,
    },
  ],
};

/**
 * Basic Auth protocol schema
 *
 * Maps to BasicAuthProtocol from crates/credential/src/protocols/basic_auth.rs
 * Used for HTTP Basic Authentication (username + password)
 */
export const BASIC_AUTH_SCHEMA: CredentialProtocolSchema = {
  kind: "basic_auth",
  displayName: "Basic Auth",
  description: "HTTP Basic Authentication with username and password",
  icon: "user",
  fields: [
    {
      name: "username",
      label: "Username",
      type: "text",
      required: true,
      placeholder: "",
      helpText: "Username for authentication",
      sensitive: false,
    },
    {
      name: "password",
      label: "Password",
      type: "password",
      required: true,
      placeholder: "",
      helpText: "Password for authentication",
      sensitive: true,
    },
  ],
};

/**
 * Database protocol schema
 *
 * Maps to DatabaseProtocol from crates/credential/src/protocols/database.rs
 * Used for database connections (PostgreSQL, MySQL, etc.)
 */
export const DATABASE_SCHEMA: CredentialProtocolSchema = {
  kind: "database",
  displayName: "Database",
  description: "Database connection credentials",
  icon: "database",
  fields: [
    {
      name: "host",
      label: "Host",
      type: "text",
      required: true,
      placeholder: "localhost",
      helpText: "Database server hostname or IP address",
      sensitive: false,
    },
    {
      name: "port",
      label: "Port",
      type: "number",
      required: false,
      placeholder: "5432",
      helpText: "Database server port (defaults to 5432 for PostgreSQL)",
      sensitive: false,
      validation: {
        min: 1,
        max: 65535,
      },
    },
    {
      name: "database",
      label: "Database",
      type: "text",
      required: true,
      placeholder: "",
      helpText: "Database name to connect to",
      sensitive: false,
    },
    {
      name: "username",
      label: "Username",
      type: "text",
      required: true,
      placeholder: "",
      helpText: "Database username",
      sensitive: false,
    },
    {
      name: "password",
      label: "Password",
      type: "password",
      required: true,
      placeholder: "",
      helpText: "Database password",
      sensitive: true,
    },
    {
      name: "ssl_mode",
      label: "SSL Mode",
      type: "select",
      required: false,
      placeholder: "disable",
      helpText: "SSL/TLS connection mode",
      sensitive: false,
      options: [
        { value: "disable", label: "Disable" },
        { value: "require", label: "Require" },
        { value: "verify-ca", label: "Verify CA" },
        { value: "verify-full", label: "Verify Full" },
      ],
    },
  ],
};

/**
 * Header Auth protocol schema
 *
 * Maps to HeaderAuthProtocol from crates/credential/src/protocols/header_auth.rs
 * Used for custom HTTP header authentication
 */
export const HEADER_AUTH_SCHEMA: CredentialProtocolSchema = {
  kind: "custom",
  displayName: "Header Auth",
  description: "Authentication using a custom HTTP header",
  icon: "code",
  fields: [
    {
      name: "header_name",
      label: "Header Name",
      type: "text",
      required: true,
      placeholder: "X-Auth-Token",
      helpText: "Name of the HTTP header to send",
      sensitive: false,
    },
    {
      name: "header_value",
      label: "Header Value",
      type: "password",
      required: true,
      placeholder: "",
      helpText: "Secret value for the header",
      sensitive: true,
    },
  ],
};

/**
 * OAuth2 protocol schema
 *
 * Maps to OAuth2Protocol from crates/credential/src/protocols/oauth2/
 * Used for OAuth2 flows (authorization code, client credentials, device code)
 */
export const OAUTH2_SCHEMA: CredentialProtocolSchema = {
  kind: "oauth2",
  displayName: "OAuth2",
  description: "OAuth2 authentication flow",
  icon: "shield",
  fields: [
    {
      name: "client_id",
      label: "Client ID",
      type: "text",
      required: true,
      placeholder: "",
      helpText: "OAuth2 application client ID",
      sensitive: false,
    },
    {
      name: "client_secret",
      label: "Client Secret",
      type: "password",
      required: true,
      placeholder: "",
      helpText: "OAuth2 application client secret",
      sensitive: true,
    },
    {
      name: "auth_url",
      label: "Authorization URL",
      type: "url",
      required: true,
      placeholder: "https://provider.com/oauth/authorize",
      helpText: "OAuth2 authorization endpoint URL",
      sensitive: false,
      validation: {
        pattern: "^https?://.*",
      },
    },
    {
      name: "token_url",
      label: "Token URL",
      type: "url",
      required: true,
      placeholder: "https://provider.com/oauth/token",
      helpText: "OAuth2 token endpoint URL",
      sensitive: false,
      validation: {
        pattern: "^https?://.*",
      },
    },
    {
      name: "scopes",
      label: "Scopes",
      type: "text",
      required: false,
      placeholder: "read,write",
      helpText: "Comma-separated list of OAuth2 scopes (optional)",
      sensitive: false,
    },
    {
      name: "grant_type",
      label: "Grant Type",
      type: "select",
      required: false,
      placeholder: "authorization_code",
      helpText: "OAuth2 grant type flow",
      sensitive: false,
      options: [
        { value: "authorization_code", label: "Authorization Code" },
        { value: "client_credentials", label: "Client Credentials" },
        { value: "device_code", label: "Device Code" },
      ],
    },
    {
      name: "auth_style",
      label: "Auth Style",
      type: "select",
      required: false,
      placeholder: "header",
      helpText: "How client credentials are sent in token request",
      sensitive: false,
      options: [
        { value: "header", label: "Authorization Header (RFC 6749)" },
        { value: "post_body", label: "POST Body (GitHub, Slack)" },
      ],
    },
    {
      name: "pkce",
      label: "Use PKCE",
      type: "checkbox",
      required: false,
      helpText: "Enable Proof Key for Code Exchange (recommended for mobile/desktop apps)",
      sensitive: false,
    },
  ],
};

/**
 * Registry of all available credential protocol schemas
 *
 * Used by the UI to:
 * - Populate protocol type selector
 * - Generate dynamic forms based on selected protocol
 * - Display protocol-specific icons and descriptions
 */
export const CREDENTIAL_SCHEMAS: CredentialProtocolSchema[] = [
  API_KEY_SCHEMA,
  BASIC_AUTH_SCHEMA,
  DATABASE_SCHEMA,
  OAUTH2_SCHEMA,
  HEADER_AUTH_SCHEMA,
];

/**
 * Get schema for a specific credential kind
 *
 * @param kind - The credential protocol kind
 * @returns The schema definition or undefined if not found
 */
export function getSchemaByKind(kind: CredentialKind): CredentialProtocolSchema | undefined {
  return CREDENTIAL_SCHEMAS.find((schema) => schema.kind === kind);
}

/**
 * Validate credential form data against its protocol schema
 *
 * @param kind - The credential protocol kind
 * @param data - The form data to validate
 * @returns Array of validation error messages (empty if valid)
 */
export function validateCredentialData(
  kind: CredentialKind,
  data: Record<string, unknown>,
): string[] {
  const schema = getSchemaByKind(kind);
  if (!schema) {
    return [`Unknown credential kind: ${kind}`];
  }

  const errors: string[] = [];

  for (const field of schema.fields) {
    const value = data[field.name];

    // Check required fields
    if (field.required && (value === undefined || value === null || value === "")) {
      errors.push(`${field.label} is required`);
      continue;
    }

    // Skip validation for optional empty fields
    if (!field.required && (value === undefined || value === null || value === "")) {
      continue;
    }

    // Type-specific validation
    if (field.type === "number" && typeof value === "string") {
      const numValue = Number(value);
      if (Number.isNaN(numValue)) {
        errors.push(`${field.label} must be a valid number`);
        continue;
      }

      if (field.validation?.min !== undefined && numValue < field.validation.min) {
        errors.push(`${field.label} must be at least ${field.validation.min}`);
      }

      if (field.validation?.max !== undefined && numValue > field.validation.max) {
        errors.push(`${field.label} must be at most ${field.validation.max}`);
      }
    }

    // URL validation
    if (field.type === "url" && typeof value === "string") {
      if (field.validation?.pattern) {
        const regex = new RegExp(field.validation.pattern);
        if (!regex.test(value)) {
          errors.push(`${field.label} must be a valid URL starting with http:// or https://`);
        }
      }
    }

    // String length validation
    if (typeof value === "string") {
      if (field.validation?.minLength && value.length < field.validation.minLength) {
        errors.push(`${field.label} must be at least ${field.validation.minLength} characters`);
      }

      if (field.validation?.maxLength && value.length > field.validation.maxLength) {
        errors.push(`${field.label} must be at most ${field.validation.maxLength} characters`);
      }
    }
  }

  return errors;
}
