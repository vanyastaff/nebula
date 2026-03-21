import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { Button } from "../../components/ui/Button";
import { Card } from "../../components/ui/Card";

export function CredentialListPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  return (
    <div className="p-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-[var(--text-primary)]">{t("credentials.title")}</h1>
        <Button variant="primary" size="sm" onClick={() => void navigate("/credentials/new")}>
          {t("credentials.add")}
        </Button>
      </div>
      <Card>
        <p className="text-sm text-[var(--text-secondary)]">{t("credentials.noCredentials")}</p>
      </Card>
    </div>
  );
}
