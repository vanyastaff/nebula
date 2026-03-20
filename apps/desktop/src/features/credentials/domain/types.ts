/**
 * Credential domain types for the desktop UI.
 *
 * Re-exports auto-generated types from bindings and defines
 * UI-specific types for rotation status, health indicators, and form state.
 */

import type {
  Credential as RawCredential,
  CredentialMetadata as RawCredentialMetadata,
  CreateCredentialRequest,
  UpdateCredentialRequest,
} from "../../../bindings";

// Re-export binding types for convenience
export type { CreateCredentialRequest, UpdateCredentialRequest };

/**
 * Rotation status indicator for UI display
 *
 * Maps to the acceptance criteria:
 * - healthy: Credential is active, not expired, no rotation needed
 * - due-for-rotation: Rotation policy indicates rotation should happen soon
 * - expired: Credential has passed its expiration time
 * - failed: Last rotation attempt failed or credential is in error state
 */
export type RotationStatus = "healthy" | "due-for-rotation" | "expired" | "failed";

/**
 * Credential status derived from metadata
 *
 * Mirrors the Rust CredentialStatus enum from crates/credential/src/core/status.rs
 */
export type CredentialStatus =
  | { type: "active" }
  | { type: "pending_interaction" }
  | { type: "error"; reason: string };

/**
 * Credential metadata with properly typed dates
 *
 * Wraps the raw metadata from bindings with parsed Date objects
 */
export interface CredentialMetadata extends Omit<RawCredentialMetadata, "createdAt" | "lastAccessed" | "lastModified" | "expiresAt"> {
  createdAt: Date;
  lastAccessed: Date | null;
  lastModified: Date;
  expiresAt: Date | null;
  ttlSeconds: number | null;
  version: number;
  tags: Record<string, string>;
}

/**
 * Credential with typed metadata and computed status
 *
 * Extends the raw credential with UI-specific fields
 */
export interface Credential extends Omit<RawCredential, "metadata"> {
  id: string;
  name: string;
  kind: string;
  state: string; // Encrypted credential data
  metadata: CredentialMetadata;
}

/**
 * UI state for credential list items
 *
 * Includes computed fields for display like rotation status and health
 */
export interface CredentialListItem extends Credential {
  rotationStatus: RotationStatus;
  credentialStatus: CredentialStatus;
}

/**
 * Form state for credential creation/editing
 *
 * Used by credential forms to track input state before submission
 */
export interface CredentialFormData {
  name: string;
  kind: string;
  state: Record<string, unknown>; // Protocol-specific fields (e.g., { apiKey: string } for API key type)
  tags: Record<string, string>;
}

/**
 * Credential protocol types
 *
 * Maps to the credential protocols defined in crates/credential/src/protocols/
 */
export type CredentialKind =
  | "api_key"
  | "oauth2"
  | "basic_auth"
  | "database"
  | "ssh_key"
  | "aws_credentials"
  | "custom";

/**
 * Field type for dynamic form generation
 */
export type FieldType =
  | "text"
  | "password"
  | "email"
  | "url"
  | "textarea"
  | "number"
  | "select"
  | "checkbox";

/**
 * Field definition for credential protocol schemas
 *
 * Used to generate type-specific forms dynamically
 */
export interface CredentialFieldDefinition {
  name: string;
  label: string;
  type: FieldType;
  required: boolean;
  placeholder?: string;
  helpText?: string;
  sensitive?: boolean; // If true, field should be masked in display
  options?: Array<{ value: string; label: string }>; // For select fields
  validation?: {
    pattern?: string;
    minLength?: number;
    maxLength?: number;
    min?: number;
    max?: number;
  };
}

/**
 * Protocol schema definition
 *
 * Defines the form structure and validation for a credential protocol type
 */
export interface CredentialProtocolSchema {
  kind: CredentialKind;
  displayName: string;
  description: string;
  icon?: string; // Icon name or path for UI display
  fields: CredentialFieldDefinition[];
}

/**
 * Normalize raw credential from bindings to typed credential
 *
 * Parses ISO date strings to Date objects
 */
export function normalizeCredential(raw: RawCredential): Credential {
  return {
    id: raw.id,
    name: raw.name,
    kind: raw.kind,
    state: raw.state,
    metadata: normalizeMetadata(raw.metadata),
  };
}

/**
 * Normalize raw metadata from bindings to typed metadata
 *
 * Parses ISO date strings to Date objects
 */
export function normalizeMetadata(raw: RawCredentialMetadata): CredentialMetadata {
  return {
    createdAt: new Date(raw.createdAt),
    lastAccessed: raw.lastAccessed ? new Date(raw.lastAccessed) : null,
    lastModified: new Date(raw.lastModified),
    expiresAt: raw.expiresAt ? new Date(raw.expiresAt) : null,
    ttlSeconds: raw.ttlSeconds,
    version: raw.version,
    tags: raw.tags,
  };
}

/**
 * Compute credential status from metadata
 *
 * Mirrors the logic from crates/credential/src/core/status.rs
 */
export function computeCredentialStatus(metadata: CredentialMetadata): CredentialStatus {
  // Check if expired
  if (metadata.expiresAt && metadata.expiresAt <= new Date()) {
    return { type: "error", reason: "credential expired" };
  }

  // Check if pending interaction (OAuth2 flows, etc.)
  if (metadata.tags["credential_status"] === "pending") {
    return { type: "pending_interaction" };
  }

  return { type: "active" };
}

/**
 * Compute rotation status from metadata
 *
 * Determines UI rotation indicator based on expiration, rotation policy, and error state
 */
export function computeRotationStatus(metadata: CredentialMetadata): RotationStatus {
  // Check for expired credential
  if (metadata.expiresAt && metadata.expiresAt <= new Date()) {
    return "expired";
  }

  // Check for error state in tags
  if (metadata.tags["rotation_status"] === "failed") {
    return "failed";
  }

  // Check if rotation is due soon (within 20% of TTL remaining)
  if (metadata.expiresAt && metadata.ttlSeconds) {
    const now = new Date();
    const timeRemaining = metadata.expiresAt.getTime() - now.getTime();
    const ttlMs = metadata.ttlSeconds * 1000;
    const threshold = ttlMs * 0.2; // 20% of TTL

    if (timeRemaining <= threshold && timeRemaining > 0) {
      return "due-for-rotation";
    }
  }

  return "healthy";
}

/**
 * Convert credential to list item with computed fields
 */
export function toListItem(credential: Credential): CredentialListItem {
  return {
    ...credential,
    rotationStatus: computeRotationStatus(credential.metadata),
    credentialStatus: computeCredentialStatus(credential.metadata),
  };
}
