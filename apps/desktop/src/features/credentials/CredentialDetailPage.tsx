import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router";
import { Button } from "../../components/ui/Button";
import { Card } from "../../components/ui/Card";

export function CredentialDetailPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  return (
    <div className="p-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-[var(--text-primary)]">
          {t("credentials.title")} — {id}
        </h1>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void navigate(`/credentials/${id}/edit`)}
          >
            {t("common.edit")}
          </Button>
          <Button variant="danger" size="sm" onClick={() => void navigate("/credentials")}>
            {t("common.back")}
          </Button>
        </div>
      </div>
      <Card>
        <p className="text-sm text-[var(--text-secondary)]">{t("credentials.noCredentials")}</p>
      </Card>
    </div>
  );
}
