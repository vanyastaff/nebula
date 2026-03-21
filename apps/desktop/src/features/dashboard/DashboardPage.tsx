import { useTranslation } from "react-i18next";
import { Card } from "../../components/ui/Card";
import { useAuthStore } from "../auth/store";

export function DashboardPage() {
  const { t } = useTranslation();
  const { user } = useAuthStore();

  const userName = user?.name ?? user?.email ?? t("dashboard.welcome");
  const greeting = user ? t("dashboard.greeting", { name: userName }) : t("dashboard.welcome");

  return (
    <div className="p-6">
      <h1 className="mb-6 text-2xl font-bold text-[var(--text-primary)]">{t("dashboard.title")}</h1>

      {/* Welcome Card */}
      <Card className="mb-6">
        <h2 className="mb-2 text-xl font-semibold text-[var(--text-primary)]">{greeting}</h2>
        <p className="text-[var(--text-secondary)]">{t("dashboard.subtitle")}</p>
      </Card>

      {/* Quick Actions Cards */}
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
        <Card header={t("dashboard.quickActions")}>
          <p className="text-sm text-[var(--text-secondary)]">
            {t("dashboard.quickActionsDescription")}
          </p>
        </Card>

        <Card header={t("dashboard.recentWorkflows")}>
          <p className="text-sm text-[var(--text-secondary)]">{t("dashboard.noWorkflows")}</p>
        </Card>

        <Card header={t("dashboard.systemStatus")}>
          <div className="flex items-center gap-2 text-sm">
            <div className="h-2 w-2 rounded-full bg-green-500" />
            <span className="text-[var(--text-secondary)]">
              {t("dashboard.allSystemsOperational")}
            </span>
          </div>
        </Card>
      </div>
    </div>
  );
}
