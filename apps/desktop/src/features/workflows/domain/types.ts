/**
 * Workflow domain types for the desktop UI.
 *
 * Defines UI-specific types for visual workflow canvas, including nodes,
 * edges, workflow metadata, status tracking, and form state.
 */

/**
 * Workflow status indicator for UI display
 *
 * Maps to workflow lifecycle states:
 * - draft: Workflow is being designed, not yet deployed
 * - active: Workflow is deployed and running on server
 * - paused: Workflow is deployed but execution is paused
 * - error: Workflow encountered an error during execution
 * - archived: Workflow is no longer active but kept for history
 */
export type WorkflowStatus = "draft" | "active" | "paused" | "error" | "archived";

/**
 * Workflow execution mode
 *
 * - manual: Workflow must be triggered manually
 * - scheduled: Workflow runs on a schedule (cron)
 * - event: Workflow triggered by external events
 */
export type WorkflowTriggerMode = "manual" | "scheduled" | "event";

/**
 * Workflow metadata with properly typed dates
 */
export interface WorkflowMetadata {
  createdAt: Date;
  lastModified: Date;
  lastDeployed: Date | null;
  lastExecuted: Date | null;
  version: number;
  tags: Record<string, string>;
  author: string | null;
  description: string | null;
}

/**
 * Raw workflow metadata from bindings (with ISO date strings)
 */
export interface RawWorkflowMetadata {
  createdAt: string;
  lastModified: string;
  lastDeployed: string | null;
  lastExecuted: string | null;
  version: number;
  tags: Record<string, string>;
  author: string | null;
  description: string | null;
}

/**
 * Position for nodes on the canvas
 */
export interface Position {
  x: number;
  y: number;
}

/**
 * Size dimensions for nodes
 */
export interface Size {
  width: number;
  height: number;
}

/**
 * Node port definition for connecting edges
 */
export interface NodePort {
  id: string;
  name: string;
  type: "input" | "output";
  dataType: string; // Type signature for validation (e.g., "string", "number", "any")
  required: boolean;
}

/**
 * Node data containing action configuration
 */
export interface NodeData {
  actionType: string; // e.g., "http.request", "data.transform"
  label: string;
  parameters: Record<string, unknown>; // Action-specific configuration
  inputs: NodePort[];
  outputs: NodePort[];
  icon?: string;
  color?: string;
  validationErrors?: string[];
}

/**
 * Visual workflow node
 */
export interface WorkflowNode {
  id: string;
  type: "action" | "trigger" | "condition" | "loop";
  position: Position;
  data: NodeData;
  selected?: boolean;
  dragging?: boolean;
}

/**
 * Edge connection between nodes
 */
export interface WorkflowEdge {
  id: string;
  source: string; // Source node ID
  sourcePort: string; // Source port ID
  target: string; // Target node ID
  targetPort: string; // Target port ID
  label?: string;
  animated?: boolean;
  selected?: boolean;
}

/**
 * Complete workflow definition
 */
export interface Workflow {
  id: string;
  name: string;
  status: WorkflowStatus;
  triggerMode: WorkflowTriggerMode;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  metadata: WorkflowMetadata;
  serverUrl?: string; // URL of deployed server instance
}

/**
 * UI state for workflow list items
 */
export interface WorkflowListItem {
  id: string;
  name: string;
  status: WorkflowStatus;
  triggerMode: WorkflowTriggerMode;
  nodeCount: number;
  lastModified: Date;
  lastExecuted: Date | null;
  tags: Record<string, string>;
}

/**
 * Form state for workflow creation/editing
 */
export interface WorkflowFormData {
  name: string;
  description: string;
  triggerMode: WorkflowTriggerMode;
  tags: Record<string, string>;
  scheduleExpression?: string; // Cron expression if triggerMode is "scheduled"
  eventSource?: string; // Event source if triggerMode is "event"
}

/**
 * Action definition from plugin registry
 *
 * Used to populate the node palette
 */
export interface ActionDefinition {
  type: string; // Fully qualified action type (e.g., "http.request")
  category: string; // Category for grouping in palette (e.g., "HTTP", "Data")
  name: string;
  description: string;
  icon?: string;
  color?: string;
  inputs: NodePort[];
  outputs: NodePort[];
  parameterSchema: Record<string, unknown>; // JSON Schema for parameter validation
}

/**
 * Canvas viewport state
 */
export interface CanvasViewport {
  x: number;
  y: number;
  zoom: number;
}

/**
 * Undo/redo operation
 */
export interface CanvasOperation {
  type: "add_node" | "remove_node" | "move_node" | "add_edge" | "remove_edge" | "update_node_data";
  timestamp: Date;
  data: unknown; // Operation-specific data for undo/redo
}

/**
 * Canvas state for the workflow editor
 */
export interface CanvasState {
  workflow: Workflow;
  viewport: CanvasViewport;
  selectedNodes: string[];
  selectedEdges: string[];
  clipboard: {
    nodes: WorkflowNode[];
    edges: WorkflowEdge[];
  } | null;
  undoStack: CanvasOperation[];
  redoStack: CanvasOperation[];
  isDirty: boolean; // Has unsaved changes
}

/**
 * Workflow deployment request
 */
export interface DeployWorkflowRequest {
  workflowId: string;
  serverUrl: string;
  overwriteExisting: boolean;
}

/**
 * Workflow deployment result
 */
export interface DeployWorkflowResult {
  success: boolean;
  deployedId: string | null;
  serverUrl: string;
  error: string | null;
}

/**
 * Normalize raw workflow metadata to typed metadata
 *
 * Parses ISO date strings to Date objects
 */
export function normalizeWorkflowMetadata(raw: RawWorkflowMetadata): WorkflowMetadata {
  return {
    createdAt: new Date(raw.createdAt),
    lastModified: new Date(raw.lastModified),
    lastDeployed: raw.lastDeployed ? new Date(raw.lastDeployed) : null,
    lastExecuted: raw.lastExecuted ? new Date(raw.lastExecuted) : null,
    version: raw.version,
    tags: raw.tags,
    author: raw.author,
    description: raw.description,
  };
}

/**
 * Normalize raw workflow from bindings to domain workflow
 */
export function normalizeWorkflow(raw: any): Workflow {
  // Convert nodes from raw format (parameters as JSON string) to domain format
  const nodes: WorkflowNode[] = (raw.nodes || []).map((node: any) => ({
    ...node,
    data: {
      ...node.data,
      parameters:
        typeof node.data.parameters === "string"
          ? JSON.parse(node.data.parameters)
          : node.data.parameters,
    },
  }));

  return {
    id: raw.id,
    name: raw.name,
    status: raw.status as WorkflowStatus,
    triggerMode: raw.triggerMode as WorkflowTriggerMode,
    nodes,
    edges: raw.edges || [],
    metadata: normalizeWorkflowMetadata(raw.metadata),
    serverUrl: raw.serverUrl,
  };
}

/**
 * Convert workflow to list item for UI display
 */
export function toWorkflowListItem(workflow: Workflow): WorkflowListItem {
  return {
    id: workflow.id,
    name: workflow.name,
    status: workflow.status,
    triggerMode: workflow.triggerMode,
    nodeCount: workflow.nodes.length,
    lastModified: workflow.metadata.lastModified,
    lastExecuted: workflow.metadata.lastExecuted,
    tags: workflow.metadata.tags,
  };
}

/**
 * Alias for toWorkflowListItem
 */
export const toListItem = toWorkflowListItem;

/**
 * Validate edge connection between two ports
 *
 * Returns true if the connection is valid based on port types
 */
export function validateEdgeConnection(
  sourcePort: NodePort,
  targetPort: NodePort,
): { valid: boolean; error?: string } {
  // Cannot connect output to output or input to input
  if (sourcePort.type === targetPort.type) {
    return {
      valid: false,
      error: `Cannot connect ${sourcePort.type} to ${targetPort.type}`,
    };
  }

  // Ensure source is output and target is input
  if (sourcePort.type === "input") {
    return {
      valid: false,
      error: "Source must be an output port",
    };
  }

  // Type compatibility check (simplified - "any" is compatible with everything)
  if (sourcePort.dataType !== "any" && targetPort.dataType !== "any") {
    if (sourcePort.dataType !== targetPort.dataType) {
      return {
        valid: false,
        error: `Type mismatch: ${sourcePort.dataType} → ${targetPort.dataType}`,
      };
    }
  }

  return { valid: true };
}

/**
 * Generate unique ID for nodes/edges
 */
export function generateId(prefix: string): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
}

/**
 * Compute workflow status from node states
 *
 * If any node has validation errors, workflow is in error state
 */
export function computeWorkflowStatus(nodes: WorkflowNode[]): WorkflowStatus {
  const hasErrors = nodes.some((node) => {
    return node.data.validationErrors && node.data.validationErrors.length > 0;
  });

  if (hasErrors) {
    return "error";
  }

  return "draft"; // Default status for new workflows
}

/**
 * Create default workflow metadata
 */
export function createDefaultMetadata(): WorkflowMetadata {
  const now = new Date();
  return {
    createdAt: now,
    lastModified: now,
    lastDeployed: null,
    lastExecuted: null,
    version: 1,
    tags: {},
    author: null,
    description: null,
  };
}

/**
 * Create empty canvas state
 */
export function createEmptyCanvasState(workflowName: string = "Untitled Workflow"): CanvasState {
  return {
    workflow: {
      id: generateId("workflow"),
      name: workflowName,
      status: "draft",
      triggerMode: "manual",
      nodes: [],
      edges: [],
      metadata: createDefaultMetadata(),
    },
    viewport: {
      x: 0,
      y: 0,
      zoom: 1,
    },
    selectedNodes: [],
    selectedEdges: [],
    clipboard: null,
    undoStack: [],
    redoStack: [],
    isDirty: false,
  };
}
