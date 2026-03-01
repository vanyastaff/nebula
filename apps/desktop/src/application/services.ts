import { AuthManager } from "../infrastructure/auth/authManager";
import { ConnectionManager } from "../infrastructure/connection/connectionManager";
import { ApiClient } from "../infrastructure/http/apiClient";
import { getConnectionDefaults } from "./connectionDefaults";

export const connectionManager = new ConnectionManager(getConnectionDefaults());
export const authManager = new AuthManager();
export const apiClient = new ApiClient(connectionManager, authManager);
