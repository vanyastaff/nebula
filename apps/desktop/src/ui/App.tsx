import { AppRouter } from "../lib/router";

/**
 * Minimal App wrapper. All routing and layout is handled by AppRouter.
 * Kept as a thin entry-point; providers live in AppProvider.
 */
export function App() {
  return <AppRouter />;
}
