import { Cloud, Database, Key, KeyRound, Lock, Settings, User } from "lucide-react";
import { type ReactNode } from "react";
import { Badge } from "../../../components/ui/Badge";
import { EmptyState } from "../../../components/ui/EmptyState";
import { formatRelativeTime } from "../../../lib/format";
import type { CredentialKind } from "../domain/types";
import { useCredentialStore } from "../store";
import { RotationIndicator } from "./RotationIndicator";

/**
 * Icon component for credential types
 */
function CredentialTypeIcon({ kind }: { kind: string }) {
  const iconMap: Record<string, ReactNode> = {
    api_key: <Key size={16} />,
    oauth2: <Lock size={16} />,
    basic_auth: <User size={16} />,
    database: <Database size={16} />,
    ssh_key: <KeyRound size={16} />,
    aws_credentials: <Cloud size={16} />,
    custom: <Settings size={16} />,
  };

  const icon = iconMap[kind as CredentialKind] ?? <Lock size={16} />;

  return (
    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--surface-secondary)] border border-[var(--border-primary)]">
      {icon}
    </div>
  );
}

const statusVariantMap: Record<string, "success" | "warning" | "error"> = {
  active: "success",
  pending_interaction: "warning",
  error: "error",
};

const statusLabelMap: Record<string, string> = {
  active: "Active",
  pending_interaction: "Pending",
  error: "Error",
};

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

export interface CredentialListProps {
  onSelect?: (credentialId: string) => void;
}

export function CredentialList({ onSelect }: CredentialListProps) {
  const { credentials, selectedCredentialId, select } = useCredentialStore();

  const handleSelect = (credentialId: string) => {
    if (onSelect) {
      onSelect(credentialId);
    } else {
      select(credentialId);
    }
  };

  if (credentials.length === 0) {
    return (
      <div className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
        <div className="border-b border-[var(--border-primary)] bg-[var(--surface-secondary)]/50 px-5 py-4">
          <h2 className="text-lg font-semibold text-[var(--text-primary)]">Credentials</h2>
          <p className="mt-1 text-sm text-[var(--text-secondary)]">No credentials yet</p>
        </div>
        <EmptyState
          icon={<Lock size={48} />}
          title="No credentials found"
          description="Create your first credential to get started"
        />
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
      <div className="border-b border-[var(--border-primary)] bg-[var(--surface-secondary)]/50 px-5 py-4">
        <h2 className="text-lg font-semibold text-[var(--text-primary)]">Credentials</h2>
        <p className="mt-1 text-sm text-[var(--text-secondary)]">
          {credentials.length} {credentials.length === 1 ? "credential" : "credentials"}
        </p>
      </div>

      <div className="flex-1 overflow-auto">
        <table className="w-full border-collapse text-sm">
          <thead className="sticky top-0 z-[1] border-b border-[var(--border-primary)] bg-[var(--surface-primary)]">
            <tr>
              <th className="w-[35%] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">Name</th>
              <th className="w-[15%] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">Type</th>
              <th className="w-[15%] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">Status</th>
              <th className="w-[20%] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">Rotation</th>
              <th className="w-[15%] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">Modified</th>
            </tr>
          </thead>
          <tbody>
            {credentials.map((credential) => {
              const isSelected = credential.id === selectedCredentialId;
              return (
                <tr
                  key={credential.id}
                  tabIndex={0}
                  className={`cursor-pointer border-b border-[var(--border-primary)]/50 transition-colors hover:bg-[var(--surface-secondary)]/50 ${
                    isSelected ? "bg-[var(--accent)]/10" : ""
                  }`}
                  onClick={() => handleSelect(credential.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      handleSelect(credential.id);
                    }
                  }}
                >
                  <td className="px-4 py-3 text-[var(--text-primary)]">
                    <div className="flex items-center gap-3">
                      <CredentialTypeIcon kind={credential.kind} />
                      <div>
                        <div className="font-medium text-[var(--text-primary)]">{credential.name}</div>
                        <div className="mt-0.5 font-mono text-[11px] text-[var(--text-tertiary)]">{credential.id.slice(0, 8)}</div>
                      </div>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-[var(--text-primary)]">{formatCredentialKind(credential.kind)}</td>
                  <td className="px-4 py-3">
                    <Badge
                      variant={statusVariantMap[credential.credentialStatus.type] ?? "error"}
                      size="sm"
                      title={credential.credentialStatus.type === "error" ? credential.credentialStatus.reason : undefined}
                    >
                      {statusLabelMap[credential.credentialStatus.type] ?? "Error"}
                    </Badge>
                  </td>
                  <td className="px-4 py-3">
                    <RotationIndicator status={credential.rotationStatus} size="sm" />
                  </td>
                  <td className="px-4 py-3 text-[var(--text-secondary)]">
                    {formatRelativeTime(credential.metadata.lastModified)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
