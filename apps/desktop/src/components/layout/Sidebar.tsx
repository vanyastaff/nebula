import { KeyRound, LayoutDashboard, LogOut, PanelLeft, Settings, Workflow } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate } from "react-router";
import { useAuthStore } from "../../features/auth/store";
import { useSettingsStore } from "../../stores/settingsStore";
import { Avatar } from "../ui/Avatar";

interface NavItem {
  labelKey: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  path: string;
}

const NAV_ITEMS: NavItem[] = [
  { labelKey: "nav.dashboard", icon: LayoutDashboard, path: "/" },
  { labelKey: "nav.workflows", icon: Workflow, path: "/workflows" },
  { labelKey: "nav.credentials", icon: KeyRound, path: "/credentials" },
  { labelKey: "nav.settings", icon: Settings, path: "/settings" },
];

export function Sidebar() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { sidebarCollapsed, toggleSidebar } = useSettingsStore();
  const { user, logout } = useAuthStore();

  const collapsed = sidebarCollapsed;
  const userName = user?.name ?? user?.email ?? "User";

  return (
    <aside
      className={`flex ${collapsed ? "w-14" : "w-56"} shrink-0 flex-col border-r border-[var(--border-primary)] bg-[var(--bg-secondary)] transition-[width] duration-300 ease-[cubic-bezier(0.4,0,0.2,1)]`}
    >
      {/* Logo */}
      <div
        className={`flex h-11 shrink-0 items-center border-b border-[var(--border-primary)] ${collapsed ? "justify-center px-0" : "gap-2.5 px-4"}`}
      >
        <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-[var(--accent)] to-violet-700 text-[10px] font-bold text-white shadow-[0_0_12px_var(--accent-glow)]">
          N
        </div>
        {!collapsed && (
          <div className="flex items-center gap-2">
            <span className="select-none text-[13px] font-semibold tracking-tight text-[var(--text-primary)]">
              Nebula
            </span>
            <span className="rounded border border-[var(--border-primary)] px-1.5 py-px text-[9px] font-semibold uppercase tracking-widest text-[var(--text-tertiary)]">
              alpha
            </span>
          </div>
        )}
      </div>

      {/* Nav */}
      <nav className="flex flex-1 flex-col gap-px p-2">
        {NAV_ITEMS.map((item) => {
          const active =
            item.path === "/" ? location.pathname === "/" : location.pathname.startsWith(item.path);
          const label = t(item.labelKey);
          return (
            <button
              key={item.path}
              type="button"
              onClick={() => void navigate(item.path)}
              title={collapsed ? label : undefined}
              className={`flex items-center gap-2.5 rounded-md px-2.5 py-2 text-[13px] transition-all duration-150 ${
                active
                  ? "bg-[var(--accent-subtle)] text-[var(--accent)] font-medium shadow-[inset_3px_0_0_var(--accent)]"
                  : "text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
              } ${collapsed ? "justify-center" : ""}`}
            >
              <item.icon size={15} className="shrink-0" />
              {!collapsed && <span>{label}</span>}
            </button>
          );
        })}
      </nav>

      {/* Bottom: user + collapse */}
      <div className="flex flex-col gap-px border-t border-[var(--border-primary)] p-2">
        {/* User */}
        <div
          className={`flex items-center gap-2.5 rounded-md px-2.5 py-1.5 ${collapsed ? "justify-center" : ""}`}
          title={collapsed ? userName : undefined}
        >
          <Avatar
            name={userName}
            src={user?.avatarUrl ?? undefined}
            size="sm"
            className="h-6 w-6 shrink-0 text-[9px] ring-1 ring-[var(--border-secondary)] ring-offset-1 ring-offset-[var(--bg-secondary)]"
          />
          {!collapsed && (
            <span className="flex-1 truncate text-[12px] text-[var(--text-secondary)]">
              {userName}
            </span>
          )}
          {!collapsed && (
            <button
              type="button"
              onClick={() => void logout()}
              title={t("sidebar.logout", "Log out")}
              className="rounded-md p-1 text-[var(--text-tertiary)] transition-colors hover:bg-[var(--error-subtle)] hover:text-[var(--error)]"
            >
              <LogOut size={13} />
            </button>
          )}
        </div>

        {/* Collapse toggle */}
        <button
          type="button"
          onClick={() => void toggleSidebar()}
          title={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
          className={`flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-[var(--text-tertiary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)] transition-colors ${collapsed ? "justify-center" : ""}`}
        >
          <PanelLeft
            size={15}
            className={`shrink-0 transition-transform duration-200 ${collapsed ? "" : "rotate-180"}`}
          />
          {!collapsed && <span className="text-[12px]">{t("sidebar.collapse")}</span>}
        </button>
      </div>
    </aside>
  );
}
