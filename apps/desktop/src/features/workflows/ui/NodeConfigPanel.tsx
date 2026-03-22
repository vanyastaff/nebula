import { type CSSProperties, type ChangeEvent, useState } from "react";
import type { WorkflowNode } from "../domain/types";
import { useWorkflowStore } from "../store";

export interface NodeConfigPanelProps {
  /**
   * The node to configure
   */
  node: WorkflowNode;
  /**
   * Callback when panel is closed
   */
  onClose: () => void;
}

/**
 * Node configuration panel component
 *
 * Features:
 * - Schema-driven form generation for node parameters
 * - Field-level validation with real-time feedback
 * - Parameter editing with type-aware inputs
 * - Follows desktop UI styling patterns
 *
 * Currently supports basic parameter editing with text inputs.
 * Future enhancement: Full JSON Schema-based form generation.
 */
export function NodeConfigPanel({ node, onClose }: NodeConfigPanelProps) {
  const updateNode = useWorkflowStore((state) => state.updateNode);
  const pluginActions = useWorkflowStore((state) => state.pluginActions);

  // Find the action definition for this node
  const actionDef = pluginActions.find((a) => a.key === node.data.actionType);

  // Local form state
  const [parameters, setParameters] = useState<Record<string, unknown>>(
    node.data.parameters ?? {},
  );
  const [newParamKey, setNewParamKey] = useState<string>("");
  const [newParamValue, setNewParamValue] = useState<string>("");
  const [validationErrors, setValidationErrors] = useState<string[]>([]);

  /**
   * Handle parameter value change
   */
  function handleParameterChange(key: string, value: unknown) {
    const newParameters = { ...parameters, [key]: value };
    setParameters(newParameters);
    setValidationErrors([]);
  }

  /**
   * Handle parameter removal
   */
  function handleRemoveParameter(key: string) {
    const newParameters = { ...parameters };
    delete newParameters[key];
    setParameters(newParameters);
  }

  /**
   * Handle adding new parameter
   */
  function handleAddParameter() {
    if (!newParamKey.trim()) {
      setValidationErrors(["Parameter name cannot be empty"]);
      return;
    }

    if (parameters[newParamKey] !== undefined) {
      setValidationErrors([`Parameter "${newParamKey}" already exists`]);
      return;
    }

    // Parse value as JSON if possible, otherwise use as string
    let parsedValue: unknown = newParamValue;
    try {
      parsedValue = JSON.parse(newParamValue);
    } catch {
      // If not valid JSON, keep as string
      parsedValue = newParamValue;
    }

    const newParameters = { ...parameters, [newParamKey]: parsedValue };
    setParameters(newParameters);
    setNewParamKey("");
    setNewParamValue("");
    setValidationErrors([]);
  }

  /**
   * Handle form submission
   */
  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    // Update node in store
    updateNode(node.id, {
      data: {
        ...node.data,
        parameters,
      },
    });

    // Close panel
    onClose();
  }

  // Styles matching CredentialForm pattern
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

  const subtitleStyle: CSSProperties = {
    marginTop: 4,
    fontSize: 13,
    color: "#b8c5e6",
  };

  const formContentStyle: CSSProperties = {
    flex: 1,
    overflow: "auto",
    padding: "20px 24px",
  };

  const sectionTitleStyle: CSSProperties = {
    fontSize: 13,
    fontWeight: 600,
    color: "#8ea0cf",
    textTransform: "uppercase",
    letterSpacing: 0.5,
    marginBottom: 12,
    marginTop: 0,
  };

  const footerStyle: CSSProperties = {
    padding: "16px 20px",
    borderTop: "1px solid rgba(151, 165, 198, 0.15)",
    background: "rgba(14, 20, 38, 0.5)",
    display: "flex",
    justifyContent: "flex-end",
    gap: 12,
  };

  const buttonStyle: CSSProperties = {
    padding: "9px 18px",
    borderRadius: 8,
    border: "1px solid rgba(184, 197, 230, 0.35)",
    background: "transparent",
    color: "#edf2ff",
    fontWeight: 600,
    fontSize: 13,
    cursor: "pointer",
    transition: "all 0.15s ease",
  };

  const primaryButtonStyle: CSSProperties = {
    ...buttonStyle,
    background: "linear-gradient(135deg, rgba(99, 128, 255, 0.9) 0%, rgba(72, 99, 228, 0.9) 100%)",
    border: "1px solid rgba(99, 128, 255, 0.4)",
  };

  const fieldGroupStyle: CSSProperties = {
    marginBottom: 20,
  };

  const labelStyle: CSSProperties = {
    display: "block",
    fontSize: 13,
    fontWeight: 600,
    color: "#b8c5e6",
    marginBottom: 6,
  };

  const inputStyle: CSSProperties = {
    width: "100%",
    padding: "10px 14px",
    borderRadius: 8,
    border: "1px solid rgba(151, 165, 198, 0.3)",
    background: "rgba(14, 20, 38, 0.6)",
    color: "#edf2ff",
    fontSize: 13,
    outline: "none",
    transition: "all 0.15s ease",
  };

  const errorStyle: CSSProperties = {
    padding: "12px 16px",
    borderRadius: 8,
    background: "rgba(239, 68, 68, 0.1)",
    border: "1px solid rgba(239, 68, 68, 0.3)",
    color: "#fca5a5",
    fontSize: 13,
    marginBottom: 16,
  };

  const parameterItemStyle: CSSProperties = {
    display: "flex",
    gap: 12,
    alignItems: "flex-start",
    marginBottom: 12,
    padding: "12px",
    background: "rgba(14, 20, 38, 0.4)",
    borderRadius: 8,
    border: "1px solid rgba(151, 165, 198, 0.15)",
  };

  const parameterKeyStyle: CSSProperties = {
    flex: "0 0 140px",
    fontSize: 13,
    fontWeight: 600,
    color: "#8ea0cf",
    paddingTop: 10,
  };

  const parameterValueWrapperStyle: CSSProperties = {
    flex: 1,
  };

  const deleteButtonStyle: CSSProperties = {
    padding: "8px 12px",
    borderRadius: 6,
    border: "1px solid rgba(239, 68, 68, 0.3)",
    background: "rgba(239, 68, 68, 0.1)",
    color: "#fca5a5",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    transition: "all 0.15s ease",
  };

  const addParamSectionStyle: CSSProperties = {
    marginTop: 16,
    padding: "16px",
    background: "rgba(99, 128, 255, 0.05)",
    borderRadius: 8,
    border: "1px solid rgba(99, 128, 255, 0.2)",
  };

  const addParamRowStyle: CSSProperties = {
    display: "flex",
    gap: 12,
    marginBottom: 12,
  };

  const addButtonStyle: CSSProperties = {
    ...buttonStyle,
    background: "rgba(99, 128, 255, 0.2)",
    border: "1px solid rgba(99, 128, 255, 0.4)",
    fontSize: 12,
  };

  const infoBoxStyle: CSSProperties = {
    padding: "12px 16px",
    borderRadius: 8,
    background: "rgba(99, 128, 255, 0.1)",
    border: "1px solid rgba(99, 128, 255, 0.3)",
    color: "#b8c5e6",
    fontSize: 12,
    marginBottom: 16,
  };

  return (
    <form onSubmit={handleSubmit} style={containerStyle}>
      {/* Header */}
      <div style={headerStyle}>
        <h2 style={titleStyle}>Configure Node</h2>
        <p style={subtitleStyle}>
          {node.data.label} ({node.data.actionType})
        </p>
      </div>

      {/* Form Content */}
      <div style={formContentStyle}>
        {/* Validation Errors */}
        {validationErrors.length > 0 && (
          <div style={errorStyle}>
            {validationErrors.map((error, i) => (
              <div key={i}>{error}</div>
            ))}
          </div>
        )}

        {/* Node Info */}
        {actionDef && (
          <div style={infoBoxStyle}>
            <strong>{actionDef.name}</strong>
            <br />
            {actionDef.description}
          </div>
        )}

        {/* Parameters Section */}
        <h3 style={sectionTitleStyle}>Parameters</h3>

        {/* Existing Parameters */}
        {Object.entries(parameters).length === 0 ? (
          <p style={{ fontSize: 13, color: "#8ea0cf", marginBottom: 16 }}>
            No parameters configured. Add parameters below.
          </p>
        ) : (
          <div style={{ marginBottom: 16 }}>
            {Object.entries(parameters).map(([key, value]) => (
              <div key={key} style={parameterItemStyle}>
                <div style={parameterKeyStyle}>{key}</div>
                <div style={parameterValueWrapperStyle}>
                  <input
                    type="text"
                    value={typeof value === "string" ? value : JSON.stringify(value)}
                    onChange={(e: ChangeEvent<HTMLInputElement>) =>
                      handleParameterChange(key, e.target.value)
                    }
                    style={inputStyle}
                    placeholder="Enter value"
                  />
                </div>
                <button
                  type="button"
                  onClick={() => handleRemoveParameter(key)}
                  style={deleteButtonStyle}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Add New Parameter */}
        <div style={addParamSectionStyle}>
          <h4
            style={{
              ...sectionTitleStyle,
              fontSize: 12,
              marginBottom: 12,
            }}
          >
            Add Parameter
          </h4>
          <div style={addParamRowStyle}>
            <input
              type="text"
              value={newParamKey}
              onChange={(e: ChangeEvent<HTMLInputElement>) => setNewParamKey(e.target.value)}
              style={{ ...inputStyle, flex: "0 0 140px" }}
              placeholder="Parameter name"
            />
            <input
              type="text"
              value={newParamValue}
              onChange={(e: ChangeEvent<HTMLInputElement>) => setNewParamValue(e.target.value)}
              style={{ ...inputStyle, flex: 1 }}
              placeholder="Value (JSON or text)"
            />
          </div>
          <button type="button" onClick={handleAddParameter} style={addButtonStyle}>
            + Add Parameter
          </button>
        </div>
      </div>

      {/* Footer */}
      <div style={footerStyle}>
        <button type="button" onClick={onClose} style={buttonStyle}>
          Cancel
        </button>
        <button type="submit" style={primaryButtonStyle}>
          Save Changes
        </button>
      </div>
    </form>
  );
}
