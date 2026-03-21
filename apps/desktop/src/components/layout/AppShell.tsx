import { Outlet } from "react-router";
import { Titlebar } from "./Titlebar";
import { Sidebar } from "./Sidebar";
import { ErrorBoundary } from "../ErrorBoundary";

export function AppShell() {
  return (
    <div className="flex flex-col h-screen w-screen overflow-hidden bg-[var(--color-bg-primary)]">
      <Titlebar />
      <div className="flex flex-row flex-1 overflow-hidden">
        <Sidebar />
        <main className="flex-1 overflow-auto">
          <ErrorBoundary>
            <Outlet />
          </ErrorBoundary>
        </main>
      </div>
    </div>
  );
}
