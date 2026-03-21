import { RouterProvider, createBrowserRouter } from "react-router";
import { AuthGuard } from "../../components/auth/AuthGuard";
import { AppShell } from "../../components/layout/AppShell";
import { LoginPage } from "../../features/auth/LoginPage";
import { CredentialDetailPage } from "../../features/credentials/CredentialDetailPage";
import { CredentialFormPage } from "../../features/credentials/CredentialFormPage";
import { CredentialListPage } from "../../features/credentials/CredentialListPage";
import { DashboardPage } from "../../features/dashboard/DashboardPage";
import { SettingsPage } from "../../features/settings/SettingsPage";
import { WorkflowEditorPage } from "../../features/workflows/WorkflowEditorPage";
import { WorkflowListPage } from "../../features/workflows/WorkflowListPage";

export const router = createBrowserRouter([
  {
    path: "/login",
    element: <LoginPage />,
  },
  {
    element: <AuthGuard />,
    children: [
      {
        element: <AppShell />,
        children: [
          { index: true, element: <DashboardPage /> },
          { path: "dashboard", element: <DashboardPage /> },
          { path: "settings", element: <SettingsPage /> },
          { path: "credentials", element: <CredentialListPage /> },
          { path: "credentials/new", element: <CredentialFormPage /> },
          { path: "credentials/:id", element: <CredentialDetailPage /> },
          { path: "credentials/:id/edit", element: <CredentialFormPage /> },
          { path: "workflows", element: <WorkflowListPage /> },
          { path: "workflows/new", element: <WorkflowEditorPage /> },
          { path: "workflows/:id", element: <WorkflowEditorPage /> },
        ],
      },
    ],
  },
]);

export function AppRouter() {
  return <RouterProvider router={router} />;
}
