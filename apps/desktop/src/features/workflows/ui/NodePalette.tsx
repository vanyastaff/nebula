import type { CSSProperties } from "react";
import { useWorkflowStore } from "../store";
import type { PluginAction } from "../../../bindings";

/**
 * Icon component for action types
 */
function ActionIcon({ action }: { action: PluginAction }) {
  const style: CSSProperties = {
    fontSize: 18,
    lineHeight: 1,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: 32,
    height: 32,
    borderRadius: 8,
    background: action.color
      ? `${action.color}15`
      : "rgba(151, 165, 198, 0.08)",
    border: action.color
      ? `1px solid ${action.color}35`
      : "1px solid rgba(151, 165, 198, 0.15)",
    flexShrink: 0,
  };

  // Use icon from action if available, otherwise use a default based on category
  const icon = action.icon ?? "⚙️";

  return <div style={style}>{icon}</div>;
}

/**
 * Format action category from group array
 */
function formatCategory(group: string[]): string {
  if (group.length === 0) return "General";
  return group.join(" > ");
}

/**
 * Group actions by their primary category
 */
function groupActionsByCategory(actions: PluginAction[]): Map<string, PluginAction[]> {
  const grouped = new Map<string, PluginAction[]>();

  for (const action of actions) {
    const category = action.group[0] ?? "General";
    const existing = grouped.get(category) ?? [];
    grouped.set(category, [...existing, action]);
  }

  // Sort categories alphabetically
  return new Map([...grouped.entries()].sort(([a], [b]) => a.localeCompare(b)));
}

export interface NodePaletteProps {
  onActionSelect?: (action: PluginAction) => void;
}

export function NodePalette({ onActionSelect }: NodePaletteProps) {
  const pluginActions = useWorkflowStore((state) => state.pluginActions);

  // Group actions by category
  const groupedActions = groupActionsByCategory(pluginActions);

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

  const contentStyle: CSSProperties = {
    flex: 1,
    overflow: "auto",
    padding: "12px 0",
  };

  const categoryStyle: CSSProperties = {
    marginBottom: 16,
  };

  const categoryHeaderStyle: CSSProperties = {
    padding: "8px 20px",
    fontSize: 11,
    fontWeight: 600,
    color: "#8ea0cf",
    textTransform: "uppercase",
    letterSpacing: 0.5,
    background: "rgba(14, 20, 38, 0.3)",
    borderBottom: "1px solid rgba(151, 165, 198, 0.1)",
  };

  const actionListStyle: CSSProperties = {
    padding: "4px 0",
  };

  const actionItemStyle: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 12,
    padding: "10px 20px",
    cursor: "pointer",
    transition: "background 0.15s ease",
    borderLeft: "3px solid transparent",
  };

  const actionItemHoverStyle: CSSProperties = {
    background: "rgba(99, 128, 255, 0.1)",
    borderLeftColor: "rgba(99, 128, 255, 0.6)",
  };

  const actionContentStyle: CSSProperties = {
    flex: 1,
    minWidth: 0,
  };

  const actionNameStyle: CSSProperties = {
    fontSize: 13,
    fontWeight: 500,
    color: "#edf2ff",
    marginBottom: 2,
  };

  const actionDescStyle: CSSProperties = {
    fontSize: 12,
    color: "#8ea0cf",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  };

  const versionBadgeStyle: CSSProperties = {
    padding: "2px 6px",
    borderRadius: 4,
    background: "rgba(151, 165, 198, 0.15)",
    fontSize: 10,
    fontWeight: 600,
    color: "#b8c5e6",
    flexShrink: 0,
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
    marginBottom: 12,
    opacity: 0.5,
  };

  const emptyTextStyle: CSSProperties = {
    fontSize: 14,
    textAlign: "center",
    maxWidth: 300,
  };

  if (pluginActions.length === 0) {
    return (
      <div style={containerStyle}>
        <div style={headerStyle}>
          <h3 style={titleStyle}>Node Palette</h3>
          <p style={countStyle}>No actions available</p>
        </div>
        <div style={emptyStyle}>
          <div style={emptyIconStyle}>🔌</div>
          <p style={emptyTextStyle}>
            No plugin actions available. Load plugins to populate the node palette.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div style={containerStyle}>
      <div style={headerStyle}>
        <h3 style={titleStyle}>Node Palette</h3>
        <p style={countStyle}>{pluginActions.length} actions available</p>
      </div>

      <div style={contentStyle}>
        {Array.from(groupedActions.entries()).map(([category, actions]) => (
          <div key={category} style={categoryStyle}>
            <div style={categoryHeaderStyle}>
              {category} ({actions.length})
            </div>
            <div style={actionListStyle}>
              {actions.map((action) => (
                <div
                  key={action.key}
                  style={actionItemStyle}
                  onMouseEnter={(e) => {
                    Object.assign(e.currentTarget.style, actionItemHoverStyle);
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.background = "transparent";
                    e.currentTarget.style.borderLeftColor = "transparent";
                  }}
                  onClick={() => onActionSelect?.(action)}
                  title={`${action.name}\n${action.description}\nKey: ${action.key}`}
                >
                  <ActionIcon action={action} />
                  <div style={actionContentStyle}>
                    <div style={actionNameStyle}>{action.name}</div>
                    <div style={actionDescStyle}>{action.description}</div>
                  </div>
                  <div style={versionBadgeStyle}>v{action.version}</div>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
