import { type CSSProperties, useEffect, useState } from "react";
import { commands } from "../../../bindings";
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
    } catch (error) {
      console.error("Failed to copy:", error);
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
    } catch (error) {
      console.error("Failed to rotate credential:", error);
      alert(error instanceof Error ? error.message : "Failed to rotate credential");
    } finally {
      setRotating(false);
    }
  };

  const handleRotateCancel = () => {
    setShowRotationDialog(false);
  };

  if (loading) {
    return (
      <div style={containerStyle}>
        <p style={{ color: "#b8c5e6", fontSize: 14 }}>Loading...</p>
      </div>
    );
  }

  if (!credential) {
    return (
      <div style={containerStyle}>
        <p style={{ color: "#ffb7b7", fontSize: 14 }}>Credential not found</p>
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
    <div style={containerStyle}>
      {/* Header with name and status */}
      <div style={headerStyle}>
        <div>
          <h2 style={titleStyle}>{credential.name}</h2>
          <p style={subtitleStyle}>{schema?.displayName ?? credential.kind}</p>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <RotationIndicator status={rotationStatus} size="md" />
          <button
            type="button"
            onClick={handleRotateClick}
            style={rotateButtonStyle}
            disabled={rotating}
          >
            {rotating ? "Rotating..." : "Rotate"}
          </button>
        </div>
      </div>

      {/* Metadata Section */}
      <section style={sectionStyle}>
        <h3 style={sectionTitleStyle}>Metadata</h3>
        <div style={metadataGridStyle}>
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
      <section style={sectionStyle}>
        <h3 style={sectionTitleStyle}>Credential Values</h3>
        {schema && Object.keys(credentialFields).length > 0 ? (
          <div style={valuesGridStyle}>
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
          <p style={{ color: "#b8c5e6", fontSize: 13, margin: 0 }}>
            Credential values are encrypted and cannot be displayed.
          </p>
        )}
      </section>

      {/* Usage Section (Placeholder) */}
      <section style={sectionStyle}>
        <h3 style={sectionTitleStyle}>Usage</h3>
        <div style={usagePanelStyle}>
          <p style={{ color: "#b8c5e6", fontSize: 13, margin: 0 }}>
            Used by <strong style={{ color: "#edf2ff" }}>0 workflows</strong>
          </p>
          <p
            style={{
              color: "#8ea0cf",
              fontSize: 12,
              margin: "8px 0 0 0",
              fontStyle: "italic",
            }}
          >
            Workflow usage tracking coming soon
          </p>
        </div>
      </section>

      {/* Tags Section */}
      {Object.keys(credential.metadata.tags).length > 0 && (
        <section style={sectionStyle}>
          <h3 style={sectionTitleStyle}>Tags</h3>
          <div style={tagsGridStyle}>
            {Object.entries(credential.metadata.tags).map(([key, value]) => (
              <div key={key} style={tagStyle}>
                <span style={tagKeyStyle}>{key}:</span>
                <span style={tagValueStyle}>{value}</span>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Rotation Confirmation Dialog */}
      {showRotationDialog && (
        <div style={dialogOverlayStyle}>
          <div style={dialogContainerStyle}>
            <h3 style={dialogTitleStyle}>Rotate Credential</h3>
            <p style={dialogMessageStyle}>
              Are you sure you want to rotate this credential? This will generate new credential
              values and increment the version number.
            </p>
            <div style={dialogActionsStyle}>
              <button
                type="button"
                onClick={handleRotateCancel}
                style={dialogCancelButtonStyle}
                disabled={rotating}
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleRotateConfirm}
                style={dialogConfirmButtonStyle}
                disabled={rotating}
              >
                {rotating ? "Rotating..." : "Confirm Rotation"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

interface MetadataRowProps {
  label: string;
  value: string;
}

function MetadataRow({ label, value }: MetadataRowProps) {
  return (
    <div style={metadataRowStyle}>
      <span style={metadataLabelStyle}>{label}</span>
      <span style={metadataValueStyle}>{value}</span>
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
    <div style={credentialFieldContainerStyle}>
      <label htmlFor={`field-${label}`} style={credentialFieldLabelStyle}>
        {label}
      </label>
      <div style={credentialFieldRowStyle}>
        <span id={`field-${label}`} style={credentialFieldValueStyle}>
          {displayValue}
        </span>
        <button
          type="button"
          onClick={onCopy}
          style={{
            ...copyButtonStyle,
            backgroundColor: copied ? "rgba(61, 167, 127, 0.2)" : "rgba(184, 197, 230, 0.1)",
            borderColor: copied ? "rgba(61, 167, 127, 0.4)" : "rgba(184, 197, 230, 0.3)",
          }}
        >
          {copied ? "Copied!" : "Copy"}
        </button>
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

// Styles following the dark theme design system from App.tsx and RotationIndicator.tsx

const containerStyle: CSSProperties = {
  width: "100%",
  maxWidth: 800,
  margin: "0 auto",
  padding: 24,
  display: "flex",
  flexDirection: "column",
  gap: 20,
};

const headerStyle: CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "flex-start",
  paddingBottom: 16,
  borderBottom: "1px solid rgba(151, 165, 198, 0.2)",
};

const titleStyle: CSSProperties = {
  margin: 0,
  fontSize: 24,
  fontWeight: 600,
  color: "#edf2ff",
  letterSpacing: 0.3,
};

const subtitleStyle: CSSProperties = {
  margin: "4px 0 0 0",
  fontSize: 14,
  color: "#b8c5e6",
};

const sectionStyle: CSSProperties = {
  background: "rgba(14, 20, 38, 0.5)",
  border: "1px solid rgba(151, 165, 198, 0.15)",
  borderRadius: 10,
  padding: 16,
};

const sectionTitleStyle: CSSProperties = {
  margin: "0 0 12px 0",
  fontSize: 14,
  fontWeight: 600,
  color: "#edf2ff",
  textTransform: "uppercase",
  letterSpacing: 0.5,
};

const metadataGridStyle: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
  gap: 12,
};

const metadataRowStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
};

const metadataLabelStyle: CSSProperties = {
  fontSize: 11,
  color: "#8ea0cf",
  textTransform: "uppercase",
  letterSpacing: 0.5,
  fontWeight: 600,
};

const metadataValueStyle: CSSProperties = {
  fontSize: 13,
  color: "#edf2ff",
  fontFamily: "'Courier New', monospace",
};

const valuesGridStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 16,
};

const credentialFieldContainerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
};

const credentialFieldLabelStyle: CSSProperties = {
  fontSize: 12,
  color: "#b8c5e6",
  fontWeight: 600,
};

const credentialFieldRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  background: "rgba(14, 20, 38, 0.7)",
  border: "1px solid rgba(151, 165, 198, 0.2)",
  borderRadius: 8,
  padding: "10px 12px",
};

const credentialFieldValueStyle: CSSProperties = {
  flex: 1,
  fontSize: 14,
  color: "#edf2ff",
  fontFamily: "'Courier New', monospace",
  letterSpacing: 0.5,
};

const copyButtonStyle: CSSProperties = {
  padding: "6px 12px",
  fontSize: 11,
  fontWeight: 600,
  color: "#edf2ff",
  border: "1px solid rgba(184, 197, 230, 0.3)",
  borderRadius: 6,
  cursor: "pointer",
  transition: "all 0.15s ease",
};

const usagePanelStyle: CSSProperties = {
  padding: 12,
  background: "rgba(14, 20, 38, 0.4)",
  border: "1px solid rgba(151, 165, 198, 0.15)",
  borderRadius: 8,
};

const tagsGridStyle: CSSProperties = {
  display: "flex",
  flexWrap: "wrap",
  gap: 8,
};

const tagStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "4px 10px",
  background: "rgba(14, 20, 38, 0.6)",
  border: "1px solid rgba(151, 165, 198, 0.25)",
  borderRadius: 6,
  fontSize: 12,
};

const tagKeyStyle: CSSProperties = {
  color: "#8ea0cf",
  fontWeight: 600,
};

const tagValueStyle: CSSProperties = {
  color: "#edf2ff",
  fontFamily: "'Courier New', monospace",
};

const rotateButtonStyle: CSSProperties = {
  padding: "8px 16px",
  fontSize: 13,
  fontWeight: 600,
  color: "#edf2ff",
  background: "rgba(61, 167, 127, 0.2)",
  border: "1px solid rgba(61, 167, 127, 0.4)",
  borderRadius: 8,
  cursor: "pointer",
  transition: "all 0.15s ease",
};

const dialogOverlayStyle: CSSProperties = {
  position: "fixed",
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  background: "rgba(0, 0, 0, 0.7)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
};

const dialogContainerStyle: CSSProperties = {
  background: "#0e1426",
  border: "1px solid rgba(151, 165, 198, 0.3)",
  borderRadius: 12,
  padding: 24,
  maxWidth: 480,
  width: "90%",
  boxShadow: "0 8px 32px rgba(0, 0, 0, 0.4)",
};

const dialogTitleStyle: CSSProperties = {
  margin: "0 0 12px 0",
  fontSize: 18,
  fontWeight: 600,
  color: "#edf2ff",
};

const dialogMessageStyle: CSSProperties = {
  margin: "0 0 24px 0",
  fontSize: 14,
  lineHeight: 1.6,
  color: "#b8c5e6",
};

const dialogActionsStyle: CSSProperties = {
  display: "flex",
  gap: 12,
  justifyContent: "flex-end",
};

const dialogCancelButtonStyle: CSSProperties = {
  padding: "10px 20px",
  fontSize: 13,
  fontWeight: 600,
  color: "#b8c5e6",
  background: "rgba(184, 197, 230, 0.1)",
  border: "1px solid rgba(184, 197, 230, 0.3)",
  borderRadius: 8,
  cursor: "pointer",
  transition: "all 0.15s ease",
};

const dialogConfirmButtonStyle: CSSProperties = {
  padding: "10px 20px",
  fontSize: 13,
  fontWeight: 600,
  color: "#edf2ff",
  background: "rgba(61, 167, 127, 0.3)",
  border: "1px solid rgba(61, 167, 127, 0.5)",
  borderRadius: 8,
  cursor: "pointer",
  transition: "all 0.15s ease",
};
