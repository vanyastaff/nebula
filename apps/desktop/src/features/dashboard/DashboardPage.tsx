import { Activity, CheckCircle2, KeyRound, Play, Workflow, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useAuthStore } from "../auth/store";

type SystemStatus = "operational" | "degraded" | "down";

function PulseDot({ status }: { status: SystemStatus }) {
  const colorClass = {
    operational: "bg-[var(--success)]",
    degraded: "bg-[var(--warning)]",
    down: "bg-[var(--error)]",
  }[status];

  return (
    <span className="relative flex h-2 w-2 shrink-0">
      <span
        className={`absolute inline-flex h-full w-full animate-ping rounded-full ${colorClass} opacity-40`}
      />
      <span className={`relative inline-flex h-2 w-2 rounded-full ${colorClass}`} />
    </span>
  );
}

interface MetricCardProps {
  label: string;
  value: string | number;
  sub?: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  variant?: "default" | "accent";
}

function MetricCard({ label, value, sub, icon: Icon, variant = "default" }: MetricCardProps) {
  const isAccent = variant === "accent";
  return (
    <div
      className={`relative overflow-hidden rounded-xl border px-5 py-4 transition-all duration-150 ${
        isAccent
          ? "border-[var(--border-accent)] bg-[var(--accent-subtle)] hover:shadow-[var(--glow-accent)]"
          : "border-[var(--border-primary)] bg-[var(--surface-primary)] hover:border-[var(--border-secondary)]"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[11px] font-semibold uppercase tracking-widest text-[var(--text-tertiary)]">
            {label}
          </p>
          <p
            className={`mt-2 text-[28px] font-semibold leading-none tracking-tight ${
              isAccent ? "text-[var(--accent)]" : "text-[var(--text-primary)]"
            }`}
          >
            {value}
          </p>
          {sub && <p className="mt-1.5 text-xs text-[var(--text-tertiary)]">{sub}</p>}
        </div>
        <div
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg ${
            isAccent
              ? "bg-[var(--accent)]/20 text-[var(--accent)]"
              : "bg-[var(--surface-secondary)] text-[var(--text-secondary)]"
          }`}
        >
          <Icon size={16} />
        </div>
      </div>
    </div>
  );
}

const SYSTEM_SERVICES: { label: string; status: SystemStatus }[] = [
  { label: "API Server", status: "operational" },
  { label: "Workflow Engine", status: "operational" },
  { label: "Credential Store", status: "operational" },
  { label: "Event Bus", status: "operational" },
];

const STATUS_LABEL: Record<SystemStatus, string> = {
  operational: "Operational",
  degraded: "Degraded",
  down: "Down",
};

const STATUS_COLOR: Record<SystemStatus, string> = {
  operational: "text-[var(--success)]",
  degraded: "text-[var(--warning)]",
  down: "text-[var(--error)]",
};

export function DashboardPage() {
  const { t } = useTranslation();
  const { user } = useAuthStore();

  const hour = new Date().getHours();
  const timeGreeting = hour < 12 ? "Good morning" : hour < 17 ? "Good afternoon" : "Good evening";
  const userName = user?.name ?? user?.email?.split("@")[0] ?? null;

  return (
    <div className="flex h-full flex-col overflow-auto bg-[var(--bg-primary)]">
      {/* Page header */}
      <div className="border-b border-[var(--border-primary)] px-6 py-5">
        <h1 className="text-[22px] font-semibold tracking-tight text-[var(--text-primary)]">
          {userName ? `${timeGreeting}, ${userName}` : t("dashboard.title")}
        </h1>
        <p className="mt-1 text-sm text-[var(--text-tertiary)]">
          {t("dashboard.subtitle", "Overview of your automation workspace")}
        </p>
      </div>

      <div className="flex-1 space-y-5 p-6">
        {/* Metric row */}
        <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
          <MetricCard
            label={t("dashboard.activeWorkflows", "Active Workflows")}
            value="0"
            sub="None running"
            icon={Workflow}
          />
          <MetricCard
            label={t("dashboard.executionsToday", "Executions Today")}
            value="0"
            sub="Last 24 hours"
            icon={Play}
          />
          <MetricCard
            label={t("dashboard.successRate", "Success Rate")}
            value="—"
            sub="No data yet"
            icon={CheckCircle2}
          />
          <MetricCard
            label={t("dashboard.credentials", "Credentials")}
            value="0"
            sub="Stored secrets"
            icon={KeyRound}
            variant="accent"
          />
        </div>

        {/* Two-column section */}
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
          {/* Recent activity */}
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
            <div className="flex items-center gap-2 border-b border-[var(--border-primary)] px-5 py-3.5">
              <Activity size={13} className="text-[var(--text-tertiary)]" />
              <span className="text-[13px] font-medium text-[var(--text-primary)]">
                {t("dashboard.recentActivity", "Recent activity")}
              </span>
            </div>
            <div className="flex flex-col items-center justify-center px-5 py-14 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--surface-secondary)]">
                <Zap size={20} className="text-[var(--text-tertiary)]" />
              </div>
              <p className="mt-3 text-[13px] font-medium text-[var(--text-secondary)]">
                {t("dashboard.noActivity", "No activity yet")}
              </p>
              <p className="mt-1 text-xs text-[var(--text-tertiary)]">
                {t("dashboard.noActivitySub", "Workflow executions will appear here")}
              </p>
            </div>
          </div>

          {/* System status */}
          <div className="overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
            <div className="border-b border-[var(--border-primary)] px-5 py-3.5">
              <span className="text-[13px] font-medium text-[var(--text-primary)]">
                {t("dashboard.systemStatus", "System status")}
              </span>
            </div>
            <div className="space-y-px px-2 py-2">
              {SYSTEM_SERVICES.map(({ label, status }) => (
                <div
                  key={label}
                  className="flex items-center justify-between rounded-lg px-3 py-2.5 transition-colors hover:bg-[var(--surface-hover)]"
                >
                  <span className="text-sm text-[var(--text-secondary)]">{label}</span>
                  <div className="flex items-center gap-2">
                    <PulseDot status={status} />
                    <span className={`text-xs font-medium ${STATUS_COLOR[status]}`}>
                      {STATUS_LABEL[status]}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* Quick actions */}
        <div className="overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
          <div className="border-b border-[var(--border-primary)] px-5 py-3.5">
            <span className="text-[13px] font-medium text-[var(--text-primary)]">
              {t("dashboard.quickActions", "Quick actions")}
            </span>
          </div>
          <div className="flex flex-wrap gap-2 p-4">
            {[
              { label: t("dashboard.newWorkflow", "New workflow"), icon: Workflow },
              { label: t("dashboard.addCredential", "Add credential"), icon: KeyRound },
            ].map(({ label, icon: Icon }) => (
              <button
                key={label}
                type="button"
                className="flex items-center gap-2 rounded-lg border border-[var(--border-primary)] bg-[var(--surface-secondary)] px-4 py-2.5 text-[13px] text-[var(--text-secondary)] transition-all hover:border-[var(--border-accent)] hover:bg-[var(--accent-subtle)] hover:text-[var(--accent)]"
              >
                <Icon size={14} />
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
