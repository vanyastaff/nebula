import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router";
import { Button } from "../../components/ui/Button";
import { Card } from "../../components/ui/Card";

export function CredentialFormPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id?: string }>();
  const navigate = useNavigate();

  const title = id ? t("credentials.edit") : t("credentials.add");

  return (
    <div className="p-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-[var(--text-primary)]">{title}</h1>
        <Button variant="secondary" size="sm" onClick={() => void navigate("/credentials")}>
          {t("common.cancel")}
        </Button>
      </div>
      <Card>
        <p className="text-sm text-[var(--text-secondary)]">{t("credentials.noCredentials")}</p>
      </Card>
    </div>
  );
}
