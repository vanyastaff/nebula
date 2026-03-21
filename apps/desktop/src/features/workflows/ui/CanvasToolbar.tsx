import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../components/ui/Button";
import { useWorkflowStore } from "../store";

export interface CanvasToolbarProps {
  onSave?: () => void;
  onLoad?: () => void;
  onDeploy?: () => void;
}

export function CanvasToolbar({ onSave, onLoad, onDeploy }: CanvasToolbarProps) {
  const { t } = useTranslation();
  const isDirty = useWorkflowStore((state) => state.canvas.isDirty);
  const currentWorkflow = useWorkflowStore((state) => state.currentWorkflow);
  const saveCanvasToWorkflow = useWorkflowStore((state) => state.saveCanvasToWorkflow);
  const loadFromFile = useWorkflowStore((state) => state.loadFromFile);

  const handleSave = async () => {
    try {
      await saveCanvasToWorkflow();
      onSave?.();
    } catch (error) {
      // Error is already set in store
    }
  };

  const handleLoad = async () => {
    try {
      await loadFromFile();
      onLoad?.();
    } catch (error) {
      // Error is already set in store
    }
  };

  const handleDeploy = () => {
    onDeploy?.();
  };

  const containerStyle: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 12,
    padding: "12px 16px",
    background: "rgba(14, 20, 38, 0.82)",
    border: "1px solid rgba(151, 165, 198, 0.2)",
    borderRadius: 14,
    marginBottom: 12,
  };

  const titleStyle: CSSProperties = {
    flex: 1,
    fontSize: 16,
    fontWeight: 600,
    color: "#edf2ff",
    letterSpacing: 0.3,
  };

  const buttonGroupStyle: CSSProperties = {
    display: "flex",
    gap: 8,
  };

  const dirtyIndicatorStyle: CSSProperties = {
    width: 8,
    height: 8,
    borderRadius: "50%",
    background: "#ffc107",
    marginRight: 4,
  };

  return (
    <div style={containerStyle}>
      <div style={titleStyle}>
        {currentWorkflow?.name ?? t("workflows.new")}
        {isDirty && <span style={dirtyIndicatorStyle} title="Unsaved changes" />}
      </div>
      <div style={buttonGroupStyle}>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void handleLoad()}
          icon="📂"
        >
          {t("common.load")}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void handleSave()}
          disabled={!isDirty || !currentWorkflow}
          icon="💾"
        >
          {t("common.save")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleDeploy}
          disabled={!currentWorkflow}
          icon="🚀"
        >
          {t("workflows.deploy")}
        </Button>
      </div>
    </div>
  );
}
