import { KeyRound, LayoutDashboard, PanelLeft, PanelLeftClose, Settings, User } from "lucide-react";
import { useLocation, useNavigate } from "react-router";
import { useSettingsStore } from "../../stores/settingsStore";

interface NavItem {
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  path: string;
}

const NAV_ITEMS: NavItem[] = [
  { label: "Dashboard", icon: LayoutDashboard, path: "/" },
  { label: "Credentials", icon: KeyRound, path: "/credentials" },
  { label: "Settings", icon: Settings, path: "/settings" },
];

export function Sidebar() {
  const navigate = useNavigate();
  const location = useLocation();
  const { sidebarCollapsed, toggleSidebar } = useSettingsStore();

  const width = sidebarCollapsed ? "w-16" : "w-60";

  return (
    <aside
      className={`flex ${width} shrink-0 flex-col border-r border-neutral-200 bg-neutral-50 transition-[width] duration-200 dark:border-neutral-700 dark:bg-neutral-850`}
    >
      <nav className="flex flex-1 flex-col gap-1 p-2">
        {NAV_ITEMS.map((item) => {
          const active = location.pathname === item.path;
          return (
            <button
              key={item.path}
              type="button"
              onClick={() => void navigate(item.path)}
              className={`flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
                active
                  ? "bg-neutral-200 text-neutral-900 dark:bg-neutral-700 dark:text-white"
                  : "text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white"
              }`}
              title={sidebarCollapsed ? item.label : undefined}
            >
              <item.icon size={20} />
              {!sidebarCollapsed && <span>{item.label}</span>}
            </button>
          );
        })}
      </nav>

      <div className="flex flex-col gap-1 border-t border-neutral-200 p-2 dark:border-neutral-700">
        <button
          type="button"
          className="flex items-center gap-3 rounded-md px-3 py-2 text-neutral-500 dark:text-neutral-400"
          title="User"
        >
          <User size={20} />
          {!sidebarCollapsed && <span className="truncate text-sm">Account</span>}
        </button>

        <button
          type="button"
          onClick={() => void toggleSidebar()}
          className="flex items-center gap-3 rounded-md px-3 py-2 text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white"
          title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {sidebarCollapsed ? (
            <PanelLeft size={20} />
          ) : (
            <>
              <PanelLeftClose size={20} />
              <span className="text-sm">Collapse</span>
            </>
          )}
        </button>
      </div>
    </aside>
  );
}
