import { KeyRound, LayoutDashboard, LogOut, PanelLeft, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate } from "react-router";
import { Avatar } from "../ui/Avatar";
import { useAuthStore } from "../../features/auth/store";
import { useSettingsStore } from "../../stores/settingsStore";

interface NavItem {
  labelKey: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  path: string;
}

const NAV_ITEMS: NavItem[] = [
  { labelKey: "nav.dashboard", icon: LayoutDashboard, path: "/" },
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
      className={`flex ${collapsed ? "w-14" : "w-56"} shrink-0 flex-col border-r border-[var(--border-primary)] bg-[var(--bg-secondary)] transition-[width] duration-200`}
    >
      {/* Logo */}
      <div
        className={`flex h-10 shrink-0 items-center border-b border-[var(--border-primary)] ${collapsed ? "justify-center px-0" : "gap-2 px-3"}`}
      >
        <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded bg-[var(--accent)] text-[10px] font-bold text-[var(--accent-text)]">
          N
        </div>
        {!collapsed && (
          <span className="select-none text-sm font-semibold text-[var(--text-primary)]">
            Nebula
          </span>
        )}
      </div>

      {/* Nav */}
      <nav className="flex flex-1 flex-col gap-0.5 p-2">
        {NAV_ITEMS.map((item) => {
          const active = item.path === "/"
            ? location.pathname === "/"
            : location.pathname.startsWith(item.path);
          const label = t(item.labelKey);
          return (
            <button
              key={item.path}
              type="button"
              onClick={() => void navigate(item.path)}
              title={collapsed ? label : undefined}
              className={`flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors ${
                active
                  ? "bg-[var(--accent-subtle)] text-[var(--accent)] font-medium"
                  : "text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
              } ${collapsed ? "justify-center" : ""}`}
            >
              <item.icon size={16} className="shrink-0" />
              {!collapsed && <span>{label}</span>}
            </button>
          );
        })}
      </nav>

      {/* Bottom: user + collapse */}
      <div className="flex flex-col gap-0.5 border-t border-[var(--border-primary)] p-2">
        {/* User */}
        <div
          className={`flex items-center gap-2.5 rounded-md px-2.5 py-1.5 ${collapsed ? "justify-center" : ""}`}
          title={collapsed ? userName : undefined}
        >
          <Avatar name={userName} src={user?.avatarUrl ?? undefined} size="sm" className="h-6 w-6 text-[10px]" />
          {!collapsed && (
            <span className="flex-1 truncate text-xs text-[var(--text-secondary)]">{userName}</span>
          )}
          {!collapsed && (
            <button
              type="button"
              onClick={() => void logout()}
              title={t("sidebar.logout", "Log out")}
              className="rounded p-0.5 text-[var(--text-tertiary)] hover:text-[var(--error)] transition-colors"
            >
              <LogOut size={14} />
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
          <PanelLeft size={16} className={`shrink-0 transition-transform duration-200 ${collapsed ? "" : "rotate-180"}`} />
          {!collapsed && <span className="text-xs">{t("sidebar.collapse")}</span>}
        </button>
      </div>
    </aside>
  );
}
