import { RouterProvider, createBrowserRouter } from "react-router";
import { AuthGuard } from "../../components/auth/AuthGuard";
import { AppShell } from "../../components/layout/AppShell";
import { LoginPage } from "../../features/auth/LoginPage";
import { CredentialDetailPage } from "../../features/credentials/CredentialDetailPage";
import { CredentialFormPage } from "../../features/credentials/CredentialFormPage";
import { CredentialListPage } from "../../features/credentials/CredentialListPage";
import { DashboardPage } from "../../features/dashboard/DashboardPage";
import { SettingsPage } from "../../features/settings/SettingsPage";

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
        ],
      },
    ],
  },
]);

export function AppRouter() {
  return <RouterProvider router={router} />;
}
