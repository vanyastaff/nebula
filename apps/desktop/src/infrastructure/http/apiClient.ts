import { AuthManager } from "../auth/authManager";
import { ConnectionManager } from "../connection/connectionManager";

export interface HealthCheckResult {
  ok: boolean;
  status: number;
  body: string;
}

export class ApiClient {
  constructor(
    private readonly connectionManager: ConnectionManager,
    private readonly authManager: AuthManager
  ) {}

  async health(): Promise<HealthCheckResult> {
    const response = await this.request("/health", { method: "GET" });
    const body = await response.text();
    return {
      ok: response.ok,
      status: response.status,
      body,
    };
  }

  async request(path: string, init: RequestInit): Promise<Response> {
    const connection = this.connectionManager.getSnapshot();
    const authHeaders = this.authManager.buildAuthHeaders();
    const headers = new Headers(init.headers ?? {});

    for (const [key, value] of Object.entries(authHeaders)) {
      headers.set(key, value);
    }

    return fetch(`${connection.activeBaseUrl}${path}`, {
      ...init,
      headers,
    });
  }
}
