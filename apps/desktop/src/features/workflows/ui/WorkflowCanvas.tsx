import { type CSSProperties, useCallback, useEffect, useMemo } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  type Node,
  type Edge,
  type NodeChange,
  type EdgeChange,
  applyNodeChanges,
  applyEdgeChanges,
  type Connection,
  addEdge,
  BackgroundVariant,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useWorkflowStore } from "../store";
import type { WorkflowNode, WorkflowEdge } from "../domain/types";
import { generateId } from "../domain/types";

/**
 * WorkflowCanvas component with React Flow integration
 *
 * Features:
 * - Visual DAG canvas with pan/zoom/grid
 * - Drag-and-drop node placement
 * - Edge drawing between nodes
 * - Syncs with Zustand store for state management
 * - Mini-map for navigation
 * - Background grid with snapping
 */
export function WorkflowCanvas() {
  // Get store state and actions
  const nodes = useWorkflowStore((state) => state.canvas.workflow.nodes);
  const edges = useWorkflowStore((state) => state.canvas.workflow.edges);
  const viewport = useWorkflowStore((state) => state.canvas.viewport);
  const setNodes = useWorkflowStore((state) => state.setNodes);
  const setEdges = useWorkflowStore((state) => state.setEdges);
  const setViewport = useWorkflowStore((state) => state.setViewport);
  const addEdgeAction = useWorkflowStore((state) => state.addEdge);

  // Convert domain nodes to React Flow nodes
  const reactFlowNodes: Node[] = useMemo(() => {
    return nodes.map((node) => ({
      id: node.id,
      type: "default",
      position: node.position,
      data: {
        label: node.data.label,
        actionType: node.data.actionType,
      },
      selected: node.selected,
      dragging: node.dragging,
    }));
  }, [nodes]);

  // Convert domain edges to React Flow edges
  const reactFlowEdges: Edge[] = useMemo(() => {
    return edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      sourceHandle: edge.sourcePort,
      targetHandle: edge.targetPort,
      label: edge.label,
      animated: edge.animated,
      selected: edge.selected,
    }));
  }, [edges]);

  // Handle node changes (position, selection, etc.)
  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      const updatedReactFlowNodes = applyNodeChanges(changes, reactFlowNodes);

      // Convert back to domain nodes
      const updatedDomainNodes: WorkflowNode[] = updatedReactFlowNodes.map((rfNode) => {
        const originalNode = nodes.find((n) => n.id === rfNode.id);
        return {
          id: rfNode.id,
          type: originalNode?.type ?? "action",
          position: rfNode.position,
          data: originalNode?.data ?? {
            actionType: "",
            label: "Node",
            parameters: {},
            inputs: [],
            outputs: [],
          },
          selected: rfNode.selected,
          dragging: rfNode.dragging,
        };
      });

      setNodes(updatedDomainNodes);
    },
    [reactFlowNodes, nodes, setNodes],
  );

  // Handle edge changes (selection, removal, etc.)
  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      const updatedReactFlowEdges = applyEdgeChanges(changes, reactFlowEdges);

      // Convert back to domain edges
      const updatedDomainEdges: WorkflowEdge[] = updatedReactFlowEdges.map((rfEdge) => {
        const originalEdge = edges.find((e) => e.id === rfEdge.id);
        return {
          id: rfEdge.id,
          source: rfEdge.source,
          sourcePort: rfEdge.sourceHandle ?? originalEdge?.sourcePort ?? "",
          target: rfEdge.target,
          targetPort: rfEdge.targetHandle ?? originalEdge?.targetPort ?? "",
          label: originalEdge?.label,
          animated: rfEdge.animated,
          selected: rfEdge.selected,
        };
      });

      setEdges(updatedDomainEdges);
    },
    [reactFlowEdges, edges, setEdges],
  );

  // Handle new edge connections
  const onConnect = useCallback(
    (connection: Connection) => {
      if (!connection.source || !connection.target) return;

      const newEdge: WorkflowEdge = {
        id: generateId("edge"),
        source: connection.source,
        sourcePort: connection.sourceHandle ?? "output",
        target: connection.target,
        targetPort: connection.targetHandle ?? "input",
        animated: false,
      };

      addEdgeAction(newEdge);
    },
    [addEdgeAction],
  );

  // Handle viewport changes (pan/zoom)
  const onMoveEnd = useCallback(
    (_event: unknown, viewport: { x: number; y: number; zoom: number }) => {
      setViewport(viewport);
    },
    [setViewport],
  );

  // Sync viewport from store to React Flow
  useEffect(() => {
    // This effect ensures the viewport is restored when loading a workflow
    // The viewport state is managed by React Flow internally, so we don't need
    // to do anything here - just having it in the store is enough
  }, [viewport]);

  // Styles
  const canvasContainerStyle: CSSProperties = {
    width: "100%",
    height: "100%",
    background: "rgba(14, 20, 38, 0.95)",
    borderRadius: 14,
    overflow: "hidden",
    border: "1px solid rgba(151, 165, 198, 0.2)",
  };

  return (
    <div style={canvasContainerStyle}>
      <ReactFlow
        nodes={reactFlowNodes}
        edges={reactFlowEdges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onMoveEnd={onMoveEnd}
        defaultViewport={viewport}
        fitView
        snapToGrid
        snapGrid={[15, 15]}
        connectionLineStyle={{ stroke: "rgba(99, 128, 255, 0.8)", strokeWidth: 2 }}
        defaultEdgeOptions={{
          style: { stroke: "rgba(99, 128, 255, 0.6)", strokeWidth: 2 },
          type: "smoothstep",
        }}
        minZoom={0.1}
        maxZoom={4}
        proOptions={{ hideAttribution: true }}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={15}
          size={1}
          color="rgba(151, 165, 198, 0.3)"
        />
        <Controls
          style={{
            background: "rgba(14, 20, 38, 0.9)",
            border: "1px solid rgba(151, 165, 198, 0.3)",
            borderRadius: 8,
          }}
        />
        <MiniMap
          style={{
            background: "rgba(14, 20, 38, 0.9)",
            border: "1px solid rgba(151, 165, 198, 0.3)",
            borderRadius: 8,
          }}
          nodeColor={(node) => {
            if (node.selected) return "rgba(99, 128, 255, 0.8)";
            return "rgba(151, 165, 198, 0.5)";
          }}
        />
      </ReactFlow>
    </div>
  );
}
