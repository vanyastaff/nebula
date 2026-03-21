import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { Button } from "../../components/ui/Button";

export function WorkflowListPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  return (
    <div className="p-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-[var(--text-primary)]">{t("workflows.title")}</h1>
        <Button variant="primary" size="sm" onClick={() => void navigate("/workflows/new")}>
          {t("workflows.add")}
        </Button>
      </div>
      <div className="text-[var(--text-secondary)]">
        {t("workflows.emptyState")}
      </div>
    </div>
  );
}
