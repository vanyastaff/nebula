import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router";
import { ReactFlowProvider } from "@xyflow/react";
import { Button } from "../../components/ui/Button";
import { WorkflowCanvas } from "./ui/WorkflowCanvas";
import { NodePalette } from "./ui/NodePalette";
import { NodeConfigPanel } from "./ui/NodeConfigPanel";
import { CanvasToolbar } from "./ui/CanvasToolbar";
import { useWorkflowStore } from "./store";

export function WorkflowEditorPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();

  // Get selected node from store
  const nodes = useWorkflowStore((state) => state.canvas.workflow.nodes);
  const selectedNode = nodes.find((n) => n.selected);
  const currentWorkflow = useWorkflowStore((state) => state.currentWorkflow);
  const deploy = useWorkflowStore((state) => state.deploy);

  const handleDeploy = async () => {
    try {
      // Prompt user for server URL
      // TODO: In the future, this could be stored in user settings or workflow metadata
      const serverUrl = currentWorkflow?.serverUrl || prompt("Enter Nebula server URL:", "http://localhost:8080");
      if (serverUrl) {
        await deploy(serverUrl);
        alert("Workflow deployed successfully!");
      }
    } catch (error) {
      alert(`Deployment failed: ${String(error)}`);
    }
  };

  return (
    <div className="p-6" style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-[var(--text-primary)]">
          {id ? t("workflows.edit") : t("workflows.new")}
        </h1>
        <Button variant="secondary" size="sm" onClick={() => void navigate("/workflows")}>
          {t("common.back")}
        </Button>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: "flex", gap: 16 }}>
        <div style={{ width: 300, flexShrink: 0 }}>
          <NodePalette />
        </div>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <CanvasToolbar onDeploy={() => void handleDeploy()} />
          <div style={{ flex: 1, minHeight: 0 }}>
            <ReactFlowProvider>
              <WorkflowCanvas />
            </ReactFlowProvider>
          </div>
        </div>
        {selectedNode && (
          <div style={{ width: 350, flexShrink: 0 }}>
            <NodeConfigPanel
              node={selectedNode}
              onClose={() => {
                // Deselect node by updating its selected state
                useWorkflowStore.getState().updateNode(selectedNode.id, { selected: false });
              }}
            />
          </div>
        )}
      </div>
    </div>
  );
}
