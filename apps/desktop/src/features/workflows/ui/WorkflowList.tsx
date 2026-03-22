import type { WorkflowStatus, WorkflowTriggerMode } from "../domain/types";
import { useWorkflowStore } from "../store";

const TRIGGER_ICON: Record<WorkflowTriggerMode, string> = {
  manual: "🎯",
  scheduled: "⏰",
  event: "⚡",
};

const TRIGGER_LABEL: Record<WorkflowTriggerMode, string> = {
  manual: "Manual",
  scheduled: "Scheduled",
  event: "Event-driven",
};

const STATUS_STYLE: Record<WorkflowStatus, { dot: string; label: string; badge: string }> = {
  draft: {
    dot: "bg-[var(--indicator-draft)]",
    label: "Draft",
    badge:
      "bg-[var(--surface-secondary)] text-[var(--text-tertiary)] border-[var(--border-primary)]",
  },
  active: {
    dot: "bg-[var(--indicator-active)]",
    label: "Active",
    badge: "bg-[var(--success)]/10 text-[var(--success)] border-[var(--success)]/20",
  },
  paused: {
    dot: "bg-[var(--indicator-paused)]",
    label: "Paused",
    badge: "bg-[var(--warning)]/10 text-[var(--warning)] border-[var(--warning)]/20",
  },
  error: {
    dot: "bg-[var(--indicator-error)]",
    label: "Error",
    badge: "bg-[var(--error)]/10 text-[var(--error)] border-[var(--error)]/20",
  },
  archived: {
    dot: "bg-[var(--indicator-archived)]",
    label: "Archived",
    badge:
      "bg-[var(--surface-secondary)] text-[var(--text-tertiary)] border-[var(--border-primary)]",
  },
};

function TriggerBadge({ mode }: { mode: WorkflowTriggerMode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-primary)] bg-[var(--surface-secondary)] px-2 py-0.5 text-xs text-[var(--text-secondary)]">
      <span>{TRIGGER_ICON[mode]}</span>
      {TRIGGER_LABEL[mode]}
    </span>
  );
}

function StatusBadge({ status }: { status: WorkflowStatus }) {
  const { dot, label, badge } = STATUS_STYLE[status];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${badge}`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      {label}
    </span>
  );
}

function formatDate(date: Date | null): string {
  if (!date) return "—";
  const diff = Date.now() - date.getTime();
  const days = Math.floor(diff / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days}d ago`;
  if (days < 30) return `${Math.floor(days / 7)}w ago`;
  if (days < 365) return `${Math.floor(days / 30)}mo ago`;
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}

export interface WorkflowListProps {
  onSelect?: (workflowId: string) => void;
}

export function WorkflowList({ onSelect }: WorkflowListProps) {
  const { workflows, selectedWorkflowId, select, delete: deleteWorkflow } = useWorkflowStore();

  const handleRowClick = (id: string) => {
    select(id);
    onSelect?.(id);
  };

  const handleDelete = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    if (window.confirm("Delete this workflow?")) {
      try {
        await deleteWorkflow(id);
      } catch (err) {
        alert(`Failed: ${String(err)}`);
      }
    }
  };

  return (
    <div className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[var(--border-primary)] px-5 py-4">
        <div>
          <h2 className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">
            Workflows
          </h2>
          <p className="mt-0.5 text-xs text-[var(--text-tertiary)]">
            {workflows.length} {workflows.length === 1 ? "workflow" : "workflows"}
          </p>
        </div>
      </div>

      {/* Empty state */}
      {workflows.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--surface-secondary)] text-[var(--text-tertiary)]">
            <span className="text-xl">📋</span>
          </div>
          <p className="text-sm font-medium text-[var(--text-secondary)]">No workflows yet</p>
          <p className="text-xs text-[var(--text-tertiary)]">
            Create your first workflow to get started
          </p>
        </div>
      ) : (
        <div className="flex-1 overflow-auto">
          <table className="w-full border-collapse text-[13px]">
            <thead className="sticky top-0 z-[1] border-b border-[var(--border-primary)] bg-[var(--surface-primary)]">
              <tr>
                <th className="w-8 px-4 py-3 text-left" />
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Name
                </th>
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Status
                </th>
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Trigger
                </th>
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Nodes
                </th>
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Modified
                </th>
                <th className="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Last Run
                </th>
                <th className="w-16 px-4 py-3 text-center text-[11px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {workflows.map((workflow) => {
                const isSelected = workflow.id === selectedWorkflowId;
                return (
                  <tr
                    key={workflow.id}
                    onClick={() => handleRowClick(workflow.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") handleRowClick(workflow.id);
                    }}
                    tabIndex={0}
                    className={`cursor-pointer border-b border-[var(--border-primary)]/60 transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)] ${
                      isSelected ? "bg-[var(--accent)]/5" : "hover:bg-[var(--surface-secondary)]"
                    }`}
                  >
                    <td className="px-4 py-3">
                      <div className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] text-sm">
                        {TRIGGER_ICON[workflow.triggerMode]}
                      </div>
                    </td>
                    <td className="px-4 py-3">
                      <div className="font-medium text-[var(--text-primary)]">{workflow.name}</div>
                      {Object.keys(workflow.tags).length > 0 && (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {Object.entries(workflow.tags)
                            .slice(0, 3)
                            .map(([k, v]) => (
                              <span
                                key={k}
                                className="rounded border border-[var(--border-primary)] bg-[var(--surface-secondary)] px-1.5 py-px text-[10px] text-[var(--text-tertiary)]"
                              >
                                {k}: {v}
                              </span>
                            ))}
                          {Object.keys(workflow.tags).length > 3 && (
                            <span className="rounded border border-[var(--border-primary)] bg-[var(--surface-secondary)] px-1.5 py-px text-[10px] text-[var(--text-tertiary)]">
                              +{Object.keys(workflow.tags).length - 3}
                            </span>
                          )}
                        </div>
                      )}
                    </td>
                    <td className="px-4 py-3">
                      <StatusBadge status={workflow.status} />
                    </td>
                    <td className="px-4 py-3">
                      <TriggerBadge mode={workflow.triggerMode} />
                    </td>
                    <td className="px-4 py-3 tabular-nums text-[var(--text-secondary)]">
                      {workflow.nodeCount}
                    </td>
                    <td className="px-4 py-3 text-[var(--text-secondary)]">
                      {formatDate(workflow.lastModified)}
                    </td>
                    <td className="px-4 py-3 text-[var(--text-secondary)]">
                      {formatDate(workflow.lastExecuted)}
                    </td>
                    <td className="px-4 py-3 text-center">
                      <button
                        type="button"
                        onClick={(e) => void handleDelete(e, workflow.id)}
                        className="rounded-md border border-[var(--error)]/20 bg-[var(--error)]/10 px-2.5 py-1 text-[11px] font-medium text-[var(--error)] transition-colors hover:bg-[var(--error)]/20 hover:border-[var(--error)]/40"
                      >
                        Delete
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
