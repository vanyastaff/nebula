import { Outlet } from "react-router";
import { ErrorBoundary } from "../ErrorBoundary";
import { Sidebar } from "./Sidebar";
import { Titlebar } from "./Titlebar";

export function AppShell() {
  return (
    <div className="flex flex-col h-screen w-screen overflow-hidden bg-[var(--bg-primary)]">
      <Titlebar />
      <div className="flex flex-row flex-1 overflow-hidden">
        <Sidebar />
        <main className="flex-1 overflow-auto transition-[margin] duration-300 ease-[cubic-bezier(0.4,0,0.2,1)]">
          <ErrorBoundary>
            <Outlet />
          </ErrorBoundary>
        </main>
      </div>
    </div>
  );
}
