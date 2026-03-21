import { RouterProvider, createBrowserRouter, useParams } from "react-router";
import { AuthGuard } from "../../components/auth/AuthGuard";
import { AppShell } from "../../components/layout/AppShell";
import { LoginPage } from "../../features/auth/LoginPage";

// --- Page placeholders (until credential feature modules are migrated) ---

function DashboardPage() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>
    </div>
  );
}

function SettingsPage() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">Settings</h1>
    </div>
  );
}

function CredentialListPage() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">Credentials</h1>
      <p className="mt-2 text-neutral-400">Manage your stored credentials.</p>
    </div>
  );
}

function CredentialDetailPage() {
  const { id } = useParams<{ id: string }>();
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">Credential Detail</h1>
      <p className="mt-2 text-neutral-400">Viewing credential {id}</p>
    </div>
  );
}

function CredentialFormPage() {
  const { id } = useParams<{ id?: string }>();
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">{id ? "Edit Credential" : "New Credential"}</h1>
    </div>
  );
}

// --- Router definition ---

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
