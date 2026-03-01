import { useSyncExternalStore } from "react";
import { AuthSnapshot } from "../domain/auth";
import { ConnectionSnapshot } from "../domain/connection";
import { authManager, connectionManager } from "./services";

export function useConnectionSnapshot(): ConnectionSnapshot {
  return useSyncExternalStore(
    (listener) => connectionManager.subscribe(listener),
    () => connectionManager.getSnapshot()
  );
}

export function useAuthSnapshot(): AuthSnapshot {
  return useSyncExternalStore(
    (listener) => authManager.subscribe(listener),
    () => authManager.getSnapshot()
  );
}
