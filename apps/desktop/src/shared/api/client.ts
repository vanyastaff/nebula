import { useAuthStore } from "../../features/auth/store";
import { useConnectionStore } from "../../features/connection/store";

export async function apiFetch(
  path: string,
  init?: RequestInit
): Promise<Response> {
  const { accessToken } = useAuthStore.getState();
  const { activeBaseUrl } = useConnectionStore.getState();

  return fetch(`${activeBaseUrl}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(accessToken ? { Authorization: `Bearer ${accessToken}` } : {}),
      ...(init?.headers ?? {}),
    },
  });
}
