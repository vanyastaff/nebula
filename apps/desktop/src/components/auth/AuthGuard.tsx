import { useEffect, useRef } from "react";
import { Navigate, Outlet, useLocation } from "react-router";
import { useAuthStore } from "../../features/auth/store";
import { Spinner } from "../ui/Spinner";

/**
 * Route guard that ensures the user is authenticated.
 * Shows a spinner while checking auth status, redirects to /login if unauthenticated.
 * Saves the intended location so the login page can redirect back after success.
 */
export function AuthGuard() {
  const location = useLocation();
  const { status, initialized, initialize } = useAuthStore();
  const initCalled = useRef(false);

  useEffect(() => {
    if (!initialized && !initCalled.current) {
      initCalled.current = true;
      initialize();
    }
  }, [initialized, initialize]);

  if (!initialized || status === "authorizing") {
    return (
      <div className="flex h-screen w-screen items-center justify-center">
        <Spinner size="lg" />
      </div>
    );
  }

  if (status === "signed_out") {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }

  return <Outlet />;
}
