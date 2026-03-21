import { createBrowserRouter, RouterProvider } from "react-router";
import { AppShell } from "../../components/layout/AppShell";
import { AuthGuard } from "../../components/auth/AuthGuard";

// Placeholder pages until real page components are created
function DashboardPage() {
  return <div className="p-6"><h1 className="text-2xl font-bold">Dashboard</h1></div>;
}

function SettingsPage() {
  return <div className="p-6"><h1 className="text-2xl font-bold">Settings</h1></div>;
}

function CredentialsPage() {
  return <div className="p-6"><h1 className="text-2xl font-bold">Credentials</h1></div>;
}

function CredentialDetailPage() {
  return <div className="p-6"><h1 className="text-2xl font-bold">Credential Detail</h1></div>;
}

function LoginPage() {
  return <div className="flex h-screen items-center justify-center"><h1 className="text-2xl font-bold">Login</h1></div>;
}

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
          { path: "credentials", element: <CredentialsPage /> },
          { path: "credentials/:id", element: <CredentialDetailPage /> },
        ],
      },
    ],
  },
]);

export function AppRouter() {
  return <RouterProvider router={router} />;
}
