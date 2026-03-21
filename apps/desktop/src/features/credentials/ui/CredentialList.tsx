import type { CSSProperties } from "react";
import { useCredentialStore } from "../store";
import { RotationIndicator } from "./RotationIndicator";
import type { CredentialKind } from "../domain/types";

/**
 * Icon component for credential types
 */
function CredentialTypeIcon({ kind }: { kind: string }) {
  const iconMap: Record<string, string> = {
    api_key: "🔑",
    oauth2: "🔐",
    basic_auth: "👤",
    database: "🗄️",
    ssh_key: "🔒",
    aws_credentials: "☁️",
    custom: "⚙️",
  };

  const icon = iconMap[kind as CredentialKind] ?? "🔐";

  const style: CSSProperties = {
    fontSize: 18,
    lineHeight: 1,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: 32,
    height: 32,
    borderRadius: 8,
    background: "rgba(151, 165, 198, 0.08)",
    border: "1px solid rgba(151, 165, 198, 0.15)",
    flexShrink: 0,
  };

  return <div style={style}>{icon}</div>;
}

/**
 * Status badge for credential status (active/pending/error)
 */
function StatusBadge({ status }: { status: { type: string; reason?: string } }) {
  const configs = {
    active: {
      label: "Active",
      color: "#9de7ca",
      backgroundColor: "rgba(61, 167, 127, 0.15)",
      borderColor: "rgba(61, 167, 127, 0.35)",
    },
    pending_interaction: {
      label: "Pending",
      color: "#ffd89c",
      backgroundColor: "rgba(255, 169, 77, 0.15)",
      borderColor: "rgba(255, 169, 77, 0.35)",
    },
    error: {
      label: "Error",
      color: "#ffb7b7",
      backgroundColor: "rgba(255, 92, 92, 0.15)",
      borderColor: "rgba(255, 92, 92, 0.35)",
    },
  };

  const config = configs[status.type as keyof typeof configs] ?? configs.error;

  const style: CSSProperties = {
    display: "inline-flex",
    alignItems: "center",
    padding: "3px 8px",
    borderRadius: 6,
    border: `1px solid ${config.borderColor}`,
    backgroundColor: config.backgroundColor,
    fontSize: 11,
    fontWeight: 600,
    color: config.color,
    letterSpacing: 0.2,
  };

  return (
    <span style={style} title={status.type === "error" ? status.reason : undefined}>
      {config.label}
    </span>
  );
}

/**
 * Format credential kind for display
 */
function formatCredentialKind(kind: string): string {
  const kindMap: Record<string, string> = {
    api_key: "API Key",
    oauth2: "OAuth 2.0",
    basic_auth: "Basic Auth",
    database: "Database",
    ssh_key: "SSH Key",
    aws_credentials: "AWS Credentials",
    custom: "Custom",
  };

  return kindMap[kind as CredentialKind] ?? kind;
}

/**
 * Format date for display
 */
function formatDate(date: Date | null): string {
  if (!date) return "—";

  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));

  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days} days ago`;
  if (days < 30) return `${Math.floor(days / 7)} weeks ago`;
  if (days < 365) return `${Math.floor(days / 30)} months ago`;

  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export interface CredentialListProps {
  onSelect?: (credentialId: string) => void;
}

export function CredentialList({ onSelect }: CredentialListProps) {
  const { credentials, selectedCredentialId, select } = useCredentialStore();

  const containerStyle: CSSProperties = {
    width: "100%",
    height: "100%",
    display: "flex",
    flexDirection: "column",
    background: "rgba(14, 20, 38, 0.82)",
    border: "1px solid rgba(151, 165, 198, 0.2)",
    borderRadius: 14,
    overflow: "hidden",
  };

  const headerStyle: CSSProperties = {
    padding: "16px 20px",
    borderBottom: "1px solid rgba(151, 165, 198, 0.15)",
    background: "rgba(14, 20, 38, 0.5)",
  };

  const titleStyle: CSSProperties = {
    margin: 0,
    fontSize: 18,
    fontWeight: 600,
    color: "#edf2ff",
    letterSpacing: 0.3,
  };

  const countStyle: CSSProperties = {
    marginTop: 4,
    fontSize: 13,
    color: "#b8c5e6",
  };

  const tableContainerStyle: CSSProperties = {
    flex: 1,
    overflow: "auto",
  };

  const tableStyle: CSSProperties = {
    width: "100%",
    borderCollapse: "collapse",
    fontSize: 13,
  };

  const theadStyle: CSSProperties = {
    position: "sticky",
    top: 0,
    background: "rgba(14, 20, 38, 0.95)",
    borderBottom: "1px solid rgba(151, 165, 198, 0.2)",
    zIndex: 1,
  };

  const thStyle: CSSProperties = {
    padding: "12px 16px",
    textAlign: "left",
    fontSize: 11,
    fontWeight: 600,
    color: "#8ea0cf",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  };

  const emptyStyle: CSSProperties = {
    flex: 1,
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    padding: 40,
    color: "#8ea0cf",
  };

  const emptyIconStyle: CSSProperties = {
    fontSize: 48,
    marginBottom: 16,
    opacity: 0.5,
  };

  const emptyTextStyle: CSSProperties = {
    fontSize: 15,
    margin: 0,
    color: "#b8c5e6",
  };

  const emptyHintStyle: CSSProperties = {
    fontSize: 13,
    marginTop: 8,
    color: "#8ea0cf",
  };

  if (credentials.length === 0) {
    return (
      <div style={containerStyle}>
        <div style={headerStyle}>
          <h2 style={titleStyle}>Credentials</h2>
          <p style={countStyle}>No credentials yet</p>
        </div>
        <div style={emptyStyle}>
          <div style={emptyIconStyle}>🔐</div>
          <p style={emptyTextStyle}>No credentials found</p>
          <p style={emptyHintStyle}>Create your first credential to get started</p>
        </div>
      </div>
    );
  }

  function getRowStyle(credentialId: string): CSSProperties {
    const isSelected = credentialId === selectedCredentialId;

    return {
      borderBottom: "1px solid rgba(151, 165, 198, 0.1)",
      cursor: "pointer",
      transition: "background-color 0.15s ease",
      background: isSelected
        ? "rgba(99, 128, 255, 0.12)"
        : "transparent",
    };
  }

  function getRowHoverStyle(): CSSProperties {
    return {
      background: "rgba(151, 165, 198, 0.06)",
    };
  }

  const tdStyle: CSSProperties = {
    padding: "12px 16px",
    color: "#edf2ff",
  };

  const nameContainerStyle: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 12,
  };

  const nameTextStyle: CSSProperties = {
    fontWeight: 500,
    color: "#edf2ff",
  };

  const idStyle: CSSProperties = {
    fontSize: 11,
    color: "#8ea0cf",
    marginTop: 2,
    fontFamily: "monospace",
  };

  return (
    <div style={containerStyle}>
      <div style={headerStyle}>
        <h2 style={titleStyle}>Credentials</h2>
        <p style={countStyle}>
          {credentials.length} {credentials.length === 1 ? "credential" : "credentials"}
        </p>
      </div>

      <div style={tableContainerStyle}>
        <table style={tableStyle}>
          <thead style={theadStyle}>
            <tr>
              <th style={{ ...thStyle, width: "35%" }}>Name</th>
              <th style={{ ...thStyle, width: "15%" }}>Type</th>
              <th style={{ ...thStyle, width: "15%" }}>Status</th>
              <th style={{ ...thStyle, width: "20%" }}>Rotation</th>
              <th style={{ ...thStyle, width: "15%" }}>Modified</th>
            </tr>
          </thead>
          <tbody>
            {credentials.map((credential) => (
              <tr
                key={credential.id}
                style={getRowStyle(credential.id)}
                onClick={() => {
                  if (onSelect) {
                    onSelect(credential.id);
                  } else {
                    select(credential.id);
                  }
                }}
                onMouseEnter={(e) => {
                  if (credential.id !== selectedCredentialId) {
                    Object.assign(e.currentTarget.style, getRowHoverStyle());
                  }
                }}
                onMouseLeave={(e) => {
                  if (credential.id !== selectedCredentialId) {
                    e.currentTarget.style.background = "transparent";
                  }
                }}
              >
                <td style={tdStyle}>
                  <div style={nameContainerStyle}>
                    <CredentialTypeIcon kind={credential.kind} />
                    <div>
                      <div style={nameTextStyle}>{credential.name}</div>
                      <div style={idStyle}>{credential.id.slice(0, 8)}</div>
                    </div>
                  </div>
                </td>
                <td style={tdStyle}>{formatCredentialKind(credential.kind)}</td>
                <td style={tdStyle}>
                  <StatusBadge status={credential.credentialStatus} />
                </td>
                <td style={tdStyle}>
                  <RotationIndicator status={credential.rotationStatus} size="sm" />
                </td>
                <td style={{ ...tdStyle, color: "#b8c5e6" }}>
                  {formatDate(credential.metadata.lastModified)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
