import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router";
import { Button } from "../../components/ui/Button";
import { WorkflowCanvas } from "./ui/WorkflowCanvas";
import { NodePalette } from "./ui/NodePalette";

export function WorkflowEditorPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();

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
        <div style={{ flex: 1, minWidth: 0 }}>
          <WorkflowCanvas />
        </div>
      </div>
    </div>
  );
}
