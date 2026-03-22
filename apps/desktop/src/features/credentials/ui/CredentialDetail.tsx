import { useEffect, useState } from "react";
import { commands } from "../../../bindings";
import { Badge } from "../../../components/ui/Badge";
import { Button } from "../../../components/ui/Button";
import { Modal } from "../../../components/ui/Modal";
import { getSchemaByKind } from "../application/schemas";
import { computeRotationStatus, normalizeCredential } from "../domain/types";
import type { Credential, CredentialKind } from "../domain/types";
import { useCredentialStore } from "../store";
import { RotationIndicator } from "./RotationIndicator";

export interface CredentialDetailProps {
  credentialId: string;
}

export function CredentialDetail({ credentialId }: CredentialDetailProps) {
  const store = useCredentialStore();
  const [credential, setCredential] = useState<Credential | null>(null);
  const [loading, setLoading] = useState(true);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const [showRotationDialog, setShowRotationDialog] = useState(false);
  const [rotating, setRotating] = useState(false);

  useEffect(() => {
    const loadCredential = async () => {
      setLoading(true);
      const cred = await store.get(credentialId);
      setCredential(cred ?? null);
      setLoading(false);
    };

    void loadCredential();
  }, [credentialId, store]);

  const handleCopy = async (fieldName: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(fieldName);
      setTimeout(() => setCopiedField(null), 2000);
    } catch {
      // Copy failed silently
    }
  };

  const handleRotateClick = () => {
    setShowRotationDialog(true);
  };

  const handleRotateConfirm = async () => {
    setRotating(true);
    try {
      const rotated = await commands.rotateCredential(credentialId);
      setCredential(normalizeCredential(rotated));
      setShowRotationDialog(false);
    } catch {
      // Rotation failed
    } finally {
      setRotating(false);
    }
  };

  const handleRotateCancel = () => {
    setShowRotationDialog(false);
  };

  if (loading) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-6">
        <p className="text-sm text-[var(--text-secondary)]">Loading...</p>
      </div>
    );
  }

  if (!credential) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-6">
        <p className="text-sm text-[var(--error)]">Credential not found</p>
      </div>
    );
  }

  // Parse credential state (encrypted in production, but we'll work with the structure)
  let credentialFields: Record<string, unknown> = {};
  try {
    credentialFields = JSON.parse(credential.state) as Record<string, unknown>;
  } catch {
    // If state is encrypted or invalid JSON, we'll show a message
    credentialFields = {};
  }

  const schema = getSchemaByKind(credential.kind as CredentialKind);
  const rotationStatus = computeRotationStatus(credential.metadata);

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-6">
      {/* Header with name and status */}
      <div className="flex items-start justify-between border-b border-[var(--border-primary)] pb-4">
        <div>
          <h2 className="text-2xl font-semibold tracking-tight text-[var(--text-primary)]">
            {credential.name}
          </h2>
          <p className="mt-1 text-sm text-[var(--text-secondary)]">
            {schema?.displayName ?? credential.kind}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <RotationIndicator status={rotationStatus} size="md" />
          <Button
            variant="secondary"
            size="md"
            onClick={handleRotateClick}
            disabled={rotating}
            loading={rotating}
          >
            {rotating ? "Rotating..." : "Rotate"}
          </Button>
        </div>
      </div>

      {/* Metadata Section */}
      <section className="rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] p-4">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-[var(--text-primary)]">
          Metadata
        </h3>
        <div className="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3">
          <MetadataRow label="ID" value={credential.id} />
          <MetadataRow label="Type" value={schema?.displayName ?? credential.kind} />
          <MetadataRow label="Created" value={formatDate(credential.metadata.createdAt)} />
          <MetadataRow label="Modified" value={formatDate(credential.metadata.lastModified)} />
          <MetadataRow
            label="Last Accessed"
            value={
              credential.metadata.lastAccessed
                ? formatDate(credential.metadata.lastAccessed)
                : "Never"
            }
          />
          <MetadataRow
            label="Expires"
            value={
              credential.metadata.expiresAt ? formatDate(credential.metadata.expiresAt) : "Never"
            }
          />
          <MetadataRow label="Version" value={String(credential.metadata.version)} />
        </div>
      </section>

      {/* Credential Values Section */}
      <section className="rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] p-4">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-[var(--text-primary)]">
          Credential Values
        </h3>
        {schema && Object.keys(credentialFields).length > 0 ? (
          <div className="flex flex-col gap-4">
            {schema.fields.map((field) => {
              const value = credentialFields[field.name];
              if (value === undefined || value === null) return null;

              return (
                <CredentialField
                  key={field.name}
                  label={field.label}
                  value={String(value)}
                  sensitive={field.sensitive ?? false}
                  copied={copiedField === field.name}
                  onCopy={() => handleCopy(field.name, String(value))}
                />
              );
            })}
          </div>
        ) : (
          <p className="m-0 text-[13px] text-[var(--text-secondary)]">
            Credential values are encrypted and cannot be displayed.
          </p>
        )}
      </section>

      {/* Usage Section (Placeholder) */}
      <section className="rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] p-4">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-[var(--text-primary)]">
          Usage
        </h3>
        <div className="rounded-lg border border-[var(--border-primary)] bg-[var(--surface-primary)] p-3">
          <p className="m-0 text-[13px] text-[var(--text-secondary)]">
            Used by <strong className="text-[var(--text-primary)]">0 workflows</strong>
          </p>
          <p className="m-0 mt-2 text-xs italic text-[var(--text-tertiary)]">
            Workflow usage tracking coming soon
          </p>
        </div>
      </section>

      {/* Tags Section */}
      {Object.keys(credential.metadata.tags).length > 0 && (
        <section className="rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] p-4">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-[var(--text-primary)]">
            Tags
          </h3>
          <div className="flex flex-wrap gap-2">
            {Object.entries(credential.metadata.tags).map(([key, value]) => (
              <Badge key={key} variant="neutral" size="md">
                <span className="font-semibold text-[var(--text-tertiary)]">{key}:</span>
                <span className="ml-1 font-mono text-[var(--text-primary)]">{value}</span>
              </Badge>
            ))}
          </div>
        </section>
      )}

      {/* Rotation Confirmation Dialog */}
      <Modal
        open={showRotationDialog}
        onClose={handleRotateCancel}
        title="Rotate Credential"
        size="sm"
      >
        <p className="mb-6 text-sm leading-relaxed text-[var(--text-secondary)]">
          Are you sure you want to rotate this credential? This will generate new credential
          values and increment the version number.
        </p>
        <div className="flex justify-end gap-3">
          <Button variant="secondary" size="md" onClick={handleRotateCancel} disabled={rotating}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleRotateConfirm}
            disabled={rotating}
            loading={rotating}
          >
            {rotating ? "Rotating..." : "Confirm Rotation"}
          </Button>
        </div>
      </Modal>
    </div>
  );
}

interface MetadataRowProps {
  label: string;
  value: string;
}

function MetadataRow({ label, value }: MetadataRowProps) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[11px] font-semibold uppercase tracking-wide text-[var(--text-tertiary)]">
        {label}
      </span>
      <span className="font-mono text-[13px] text-[var(--text-primary)]">{value}</span>
    </div>
  );
}

interface CredentialFieldProps {
  label: string;
  value: string;
  sensitive: boolean;
  copied: boolean;
  onCopy: () => void;
}

function CredentialField({ label, value, sensitive, copied, onCopy }: CredentialFieldProps) {
  const displayValue = sensitive ? "*".repeat(Math.min(value.length, 20)) : value;

  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={`field-${label}`}
        className="text-xs font-semibold text-[var(--text-secondary)]"
      >
        {label}
      </label>
      <div className="flex items-center gap-2 rounded-lg border border-[var(--border-primary)] bg-[var(--surface-primary)] px-3 py-2.5">
        <span
          id={`field-${label}`}
          className="flex-1 font-mono text-sm tracking-wide text-[var(--text-primary)]"
        >
          {displayValue}
        </span>
        <Button variant={copied ? "primary" : "ghost"} size="sm" onClick={onCopy}>
          {copied ? "Copied!" : "Copy"}
        </Button>
      </div>
    </div>
  );
}

function formatDate(date: Date): string {
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHours = Math.floor(diffMs / 3600000);
  const diffDays = Math.floor(diffMs / 86400000);

  if (diffMins < 1) return "just now";
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffHours < 24) return `${diffHours}h ago`;
  if (diffDays < 7) return `${diffDays}d ago`;

  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: date.getFullYear() !== now.getFullYear() ? "numeric" : undefined,
  });
}
