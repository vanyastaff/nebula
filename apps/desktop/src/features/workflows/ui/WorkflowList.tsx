import type { CSSProperties } from "react";
import type { WorkflowStatus, WorkflowTriggerMode } from "../domain/types";
import { useWorkflowStore } from "../store";

/**
 * Icon component for workflow trigger modes
 */
function TriggerModeIcon({ mode }: { mode: WorkflowTriggerMode }) {
  const iconMap: Record<WorkflowTriggerMode, string> = {
    manual: "🎯",
    scheduled: "⏰",
    event: "⚡",
  };

  const icon = iconMap[mode];

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
 * Status badge for workflow status
 */
function StatusBadge({ status }: { status: WorkflowStatus }) {
  const configs = {
    draft: {
      label: "Draft",
      color: "#b8c5e6",
      backgroundColor: "rgba(151, 165, 198, 0.15)",
      borderColor: "rgba(151, 165, 198, 0.35)",
    },
    active: {
      label: "Active",
      color: "#9de7ca",
      backgroundColor: "rgba(61, 167, 127, 0.15)",
      borderColor: "rgba(61, 167, 127, 0.35)",
    },
    paused: {
      label: "Paused",
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
    archived: {
      label: "Archived",
      color: "#8ea0cf",
      backgroundColor: "rgba(142, 160, 207, 0.15)",
      borderColor: "rgba(142, 160, 207, 0.35)",
    },
  };

  const config = configs[status];

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

  return <span style={style}>{config.label}</span>;
}

/**
 * Format trigger mode for display
 */
function formatTriggerMode(mode: WorkflowTriggerMode): string {
  const modeMap: Record<WorkflowTriggerMode, string> = {
    manual: "Manual",
    scheduled: "Scheduled",
    event: "Event-driven",
  };

  return modeMap[mode];
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

export interface WorkflowListProps {
  onSelect?: (workflowId: string) => void;
}

export function WorkflowList({ onSelect }: WorkflowListProps) {
  const { workflows, selectedWorkflowId, select, delete: deleteWorkflow } = useWorkflowStore();

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
    fontSize: 14,
  };

  const emptyIconStyle: CSSProperties = {
    fontSize: 48,
    marginBottom: 16,
    opacity: 0.5,
  };

  const handleRowClick = (workflowId: string) => {
    select(workflowId);
    if (onSelect) {
      onSelect(workflowId);
    }
  };

  const handleDelete = async (e: React.MouseEvent, workflowId: string) => {
    e.stopPropagation();

    if (window.confirm("Are you sure you want to delete this workflow?")) {
      try {
        await deleteWorkflow(workflowId);
      } catch (error) {
        alert(`Failed to delete workflow: ${String(error)}`);
      }
    }
  };

  return (
    <div style={containerStyle}>
      <div style={headerStyle}>
        <h2 style={titleStyle}>Workflows</h2>
        <div style={countStyle}>
          {workflows.length} {workflows.length === 1 ? "workflow" : "workflows"}
        </div>
      </div>

      {workflows.length === 0 ? (
        <div style={emptyStyle}>
          <div style={emptyIconStyle}>📋</div>
          <div>No workflows yet</div>
          <div style={{ marginTop: 8, fontSize: 12, color: "#8ea0cf", opacity: 0.7 }}>
            Create your first workflow to get started
          </div>
        </div>
      ) : (
        <div style={tableContainerStyle}>
          <table style={tableStyle}>
            <thead style={theadStyle}>
              <tr>
                <th style={{ ...thStyle, width: 40 }}></th>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>Status</th>
                <th style={thStyle}>Trigger</th>
                <th style={thStyle}>Nodes</th>
                <th style={thStyle}>Last Modified</th>
                <th style={thStyle}>Last Executed</th>
                <th style={{ ...thStyle, width: 100, textAlign: "center" }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {workflows.map((workflow) => {
                const isSelected = workflow.id === selectedWorkflowId;

                const rowStyle: CSSProperties = {
                  cursor: "pointer",
                  background: isSelected
                    ? "rgba(99, 128, 255, 0.08)"
                    : "transparent",
                  transition: "background 0.15s ease",
                  borderBottom: "1px solid rgba(151, 165, 198, 0.08)",
                };

                const tdStyle: CSSProperties = {
                  padding: "12px 16px",
                  color: "#edf2ff",
                };

                const nameStyle: CSSProperties = {
                  fontWeight: 500,
                  color: "#edf2ff",
                  marginBottom: 4,
                };

                const tagsStyle: CSSProperties = {
                  display: "flex",
                  gap: 6,
                  flexWrap: "wrap",
                  marginTop: 4,
                };

                const tagStyle: CSSProperties = {
                  display: "inline-block",
                  padding: "2px 6px",
                  fontSize: 10,
                  fontWeight: 500,
                  color: "#8ea0cf",
                  background: "rgba(151, 165, 198, 0.1)",
                  border: "1px solid rgba(151, 165, 198, 0.2)",
                  borderRadius: 4,
                };

                const actionButtonStyle: CSSProperties = {
                  padding: "4px 8px",
                  fontSize: 11,
                  fontWeight: 500,
                  color: "#edf2ff",
                  background: "rgba(151, 165, 198, 0.1)",
                  border: "1px solid rgba(151, 165, 198, 0.2)",
                  borderRadius: 6,
                  cursor: "pointer",
                  transition: "all 0.15s ease",
                };

                const deleteButtonStyle: CSSProperties = {
                  ...actionButtonStyle,
                  color: "#ffb7b7",
                  background: "rgba(255, 92, 92, 0.1)",
                  borderColor: "rgba(255, 92, 92, 0.2)",
                };

                return (
                  <tr
                    key={workflow.id}
                    style={rowStyle}
                    onClick={() => handleRowClick(workflow.id)}
                    onMouseEnter={(e) => {
                      if (!isSelected) {
                        e.currentTarget.style.background = "rgba(151, 165, 198, 0.05)";
                      }
                    }}
                    onMouseLeave={(e) => {
                      if (!isSelected) {
                        e.currentTarget.style.background = "transparent";
                      }
                    }}
                  >
                    <td style={tdStyle}>
                      <TriggerModeIcon mode={workflow.triggerMode} />
                    </td>
                    <td style={tdStyle}>
                      <div style={nameStyle}>{workflow.name}</div>
                      {Object.keys(workflow.tags).length > 0 && (
                        <div style={tagsStyle}>
                          {Object.entries(workflow.tags)
                            .slice(0, 3)
                            .map(([key, value]) => (
                              <span key={key} style={tagStyle}>
                                {key}: {value}
                              </span>
                            ))}
                          {Object.keys(workflow.tags).length > 3 && (
                            <span style={tagStyle}>
                              +{Object.keys(workflow.tags).length - 3} more
                            </span>
                          )}
                        </div>
                      )}
                    </td>
                    <td style={tdStyle}>
                      <StatusBadge status={workflow.status} />
                    </td>
                    <td style={tdStyle}>
                      <span style={{ color: "#b8c5e6" }}>
                        {formatTriggerMode(workflow.triggerMode)}
                      </span>
                    </td>
                    <td style={tdStyle}>
                      <span style={{ color: "#b8c5e6" }}>{workflow.nodeCount}</span>
                    </td>
                    <td style={tdStyle}>
                      <span style={{ color: "#b8c5e6" }}>
                        {formatDate(workflow.lastModified)}
                      </span>
                    </td>
                    <td style={tdStyle}>
                      <span style={{ color: "#b8c5e6" }}>
                        {formatDate(workflow.lastExecuted)}
                      </span>
                    </td>
                    <td style={{ ...tdStyle, textAlign: "center" }}>
                      <div style={{ display: "flex", gap: 6, justifyContent: "center" }}>
                        <button
                          style={deleteButtonStyle}
                          onClick={(e) => void handleDelete(e, workflow.id)}
                          onMouseEnter={(e) => {
                            e.currentTarget.style.background = "rgba(255, 92, 92, 0.2)";
                            e.currentTarget.style.borderColor = "rgba(255, 92, 92, 0.4)";
                          }}
                          onMouseLeave={(e) => {
                            e.currentTarget.style.background = "rgba(255, 92, 92, 0.1)";
                            e.currentTarget.style.borderColor = "rgba(255, 92, 92, 0.2)";
                          }}
                        >
                          Delete
                        </button>
                      </div>
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
