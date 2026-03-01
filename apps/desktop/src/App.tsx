import { useState } from "react";
import { apiBaseUrl, profile } from "./config";

export function App() {
  const [health, setHealth] = useState<string>("not checked");

  async function checkHealth() {
    try {
      const response = await fetch(`${apiBaseUrl}/health`, { method: "GET" });
      const text = await response.text();
      setHealth(response.ok ? text : `error: ${response.status}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : "unknown error";
      setHealth(`unreachable: ${message}`);
    }
  }

  return (
    <main style={{ fontFamily: "system-ui, -apple-system, Segoe UI, sans-serif", padding: 24 }}>
      <h1>Nebula Desktop</h1>
      <p>Tauri shell scaffold is ready.</p>
      <p>Profile: <strong>{profile}</strong></p>
      <p>API: <code>{apiBaseUrl}</code></p>
      <button onClick={checkHealth} style={{ padding: "8px 12px", cursor: "pointer" }}>
        Check /health
      </button>
      <p>Health: {health}</p>
    </main>
  );
}
