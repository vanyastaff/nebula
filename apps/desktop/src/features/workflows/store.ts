import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import {
  type CreateWorkflowRequest,
  type UpdateWorkflowRequest,
  type Workflow as RawWorkflow,
  type WorkflowNode as RawWorkflowNode,
  type WorkflowEdge as RawWorkflowEdge,
  type PluginAction,
  commands,
} from "../../bindings";
import {
  type Workflow,
  type WorkflowListItem,
  type WorkflowNode,
  type WorkflowEdge,
  type CanvasState,
  type CanvasViewport,
  normalizeWorkflow,
  toListItem,
  createEmptyCanvasState,
} from "./domain/types";

interface WorkflowState {
  workflows: WorkflowListItem[];
  selectedWorkflowId: string | null;
  currentWorkflow: Workflow | null;
  canvas: CanvasState;
  pluginActions: PluginAction[];
  initialized: boolean;
  error?: string;
}

interface WorkflowActions {
  initialize: () => Promise<void>;
  list: () => Promise<void>;
  get: (id: string) => Promise<Workflow | undefined>;
  create: (request: CreateWorkflowRequest) => Promise<Workflow>;
  update: (id: string, request: UpdateWorkflowRequest) => Promise<Workflow>;
  delete: (id: string) => Promise<void>;
  select: (id: string | null) => void;
  loadPluginActions: () => Promise<void>;

  // Canvas operations
  setNodes: (nodes: WorkflowNode[]) => void;
  setEdges: (edges: WorkflowEdge[]) => void;
  addNode: (node: WorkflowNode) => void;
  updateNode: (id: string, updates: Partial<WorkflowNode>) => void;
  deleteNode: (id: string) => void;
  addEdge: (edge: WorkflowEdge) => void;
  deleteEdge: (id: string) => void;
  setViewport: (viewport: CanvasViewport) => void;
  loadWorkflow: (workflow: Workflow) => void;
  clearCanvas: () => void;
  saveCanvasToWorkflow: () => Promise<void>;
}

export const useWorkflowStore = create<WorkflowState & WorkflowActions>((set, get) => ({
  workflows: [],
  selectedWorkflowId: null,
  currentWorkflow: null,
  canvas: createEmptyCanvasState(),
  pluginActions: [],
  initialized: false,

  initialize: async () => {
    try {
      const rawWorkflows = await commands.listWorkflows();
      const workflows = rawWorkflows.map((raw) => toListItem(normalizeWorkflow(raw)));

      // Load plugin actions
      const pluginActions = await commands.listPluginActions();

      set({ workflows, pluginActions, initialized: true, error: undefined });

      // Listen for workflow events from Rust backend
      await listen<RawWorkflow>("workflow_created", (event) => {
        const newWorkflow = toListItem(normalizeWorkflow(event.payload));
        set((state) => ({
          workflows: [...state.workflows, newWorkflow],
          error: undefined,
        }));
      });

      await listen<RawWorkflow>("workflow_updated", (event) => {
        const updatedWorkflow = toListItem(normalizeWorkflow(event.payload));
        set((state) => ({
          workflows: state.workflows.map((w) =>
            w.id === updatedWorkflow.id ? updatedWorkflow : w,
          ),
          currentWorkflow:
            state.currentWorkflow?.id === updatedWorkflow.id
              ? normalizeWorkflow(event.payload)
              : state.currentWorkflow,
          error: undefined,
        }));
      });

      await listen<string>("workflow_deleted", (event) => {
        const deletedId = event.payload;
        set((state) => ({
          workflows: state.workflows.filter((w) => w.id !== deletedId),
          selectedWorkflowId:
            state.selectedWorkflowId === deletedId ? null : state.selectedWorkflowId,
          currentWorkflow:
            state.currentWorkflow?.id === deletedId ? null : state.currentWorkflow,
          error: undefined,
        }));
      });
    } catch (error) {
      set({ error: String(error), initialized: true });
    }
  },

  list: async () => {
    try {
      const rawWorkflows = await commands.listWorkflows();
      const workflows = rawWorkflows.map((raw) => toListItem(normalizeWorkflow(raw)));
      set({ workflows, error: undefined });
    } catch (error) {
      set({ error: String(error) });
    }
  },

  get: async (id: string) => {
    try {
      const result = await commands.getWorkflow(id);
      if (result.status === "ok") {
        const workflow = normalizeWorkflow(result.data);
        set({ error: undefined });
        return workflow;
      }
      set({ error: result.error });
      return undefined;
    } catch (error) {
      set({ error: String(error) });
      return undefined;
    }
  },

  create: async (request: CreateWorkflowRequest) => {
    try {
      const result = await commands.createWorkflow(request);
      if (result.status === "ok") {
        const workflow = normalizeWorkflow(result.data);
        set({ error: undefined });
        // Event listener will update the list automatically
        return workflow;
      }
      set({ error: result.error });
      throw new Error(result.error);
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  update: async (id: string, request: UpdateWorkflowRequest) => {
    try {
      const result = await commands.updateWorkflow(id, request);
      if (result.status === "ok") {
        const workflow = normalizeWorkflow(result.data);
        set({ error: undefined });
        // Event listener will update the list automatically
        return workflow;
      }
      set({ error: result.error });
      throw new Error(result.error);
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  delete: async (id: string) => {
    try {
      const result = await commands.deleteWorkflow(id);
      if (result.status === "ok") {
        set({ error: undefined, selectedWorkflowId: null });
        // Event listener will update the list automatically
      } else {
        set({ error: result.error });
        throw new Error(result.error);
      }
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  select: (id: string | null) => {
    set({ selectedWorkflowId: id });
  },

  loadPluginActions: async () => {
    try {
      const pluginActions = await commands.listPluginActions();
      set({ pluginActions, error: undefined });
    } catch (error) {
      set({ error: String(error) });
    }
  },

  // Canvas operations
  setNodes: (nodes: WorkflowNode[]) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: { ...state.canvas.workflow, nodes },
        isDirty: true,
      },
    }));
  },

  setEdges: (edges: WorkflowEdge[]) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: { ...state.canvas.workflow, edges },
        isDirty: true,
      },
    }));
  },

  addNode: (node: WorkflowNode) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: {
          ...state.canvas.workflow,
          nodes: [...state.canvas.workflow.nodes, node],
        },
        isDirty: true,
      },
    }));
  },

  updateNode: (id: string, updates: Partial<WorkflowNode>) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: {
          ...state.canvas.workflow,
          nodes: state.canvas.workflow.nodes.map((n) =>
            n.id === id ? { ...n, ...updates } : n,
          ),
        },
        isDirty: true,
      },
    }));
  },

  deleteNode: (id: string) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: {
          ...state.canvas.workflow,
          nodes: state.canvas.workflow.nodes.filter((n) => n.id !== id),
          edges: state.canvas.workflow.edges.filter(
            (e) => e.source !== id && e.target !== id,
          ),
        },
        isDirty: true,
      },
    }));
  },

  addEdge: (edge: WorkflowEdge) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: {
          ...state.canvas.workflow,
          edges: [...state.canvas.workflow.edges, edge],
        },
        isDirty: true,
      },
    }));
  },

  deleteEdge: (id: string) => {
    set((state) => ({
      canvas: {
        ...state.canvas,
        workflow: {
          ...state.canvas.workflow,
          edges: state.canvas.workflow.edges.filter((e) => e.id !== id),
        },
        isDirty: true,
      },
    }));
  },

  setViewport: (viewport: CanvasViewport) => {
    set((state) => ({
      canvas: { ...state.canvas, viewport },
    }));
  },

  loadWorkflow: (workflow: Workflow) => {
    set({
      currentWorkflow: workflow,
      selectedWorkflowId: workflow.id,
      canvas: {
        ...createEmptyCanvasState(),
        workflow,
      },
    });
  },

  clearCanvas: () => {
    set({
      canvas: createEmptyCanvasState(),
      currentWorkflow: null,
      selectedWorkflowId: null,
    });
  },

  saveCanvasToWorkflow: async () => {
    const state = get();
    if (!state.currentWorkflow) {
      throw new Error("No workflow loaded");
    }

    try {
      // Convert domain nodes to raw nodes (serialize parameters to JSON string)
      const rawNodes = state.canvas.workflow.nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          parameters: JSON.stringify(node.data.parameters),
        },
      })) as unknown as RawWorkflowNode[];

      const rawEdges = state.canvas.workflow.edges as unknown as RawWorkflowEdge[];

      await get().update(state.currentWorkflow.id, {
        name: null,
        description: null,
        status: null,
        triggerMode: null,
        nodes: rawNodes,
        edges: rawEdges,
        tags: null,
        serverUrl: null,
      });
      set((s) => ({ canvas: { ...s.canvas, isDirty: false } }));
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },
}));
