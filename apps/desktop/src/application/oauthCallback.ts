import { AuthProvider } from "../domain/auth";
import { authManager, connectionManager } from "./services";

function parseProvider(value: string | null): AuthProvider | undefined {
  if (value === "google" || value === "github") {
    return value;
  }
  return undefined;
}

async function handleDeepLinkUrl(rawUrl: string): Promise<void> {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return;
  }

  if (parsed.protocol !== "nebula:" || parsed.hostname !== "auth") {
    return;
  }

  const path = parsed.pathname.replace(/\/+$/, "");
  if (path !== "/callback") {
    return;
  }

  const accessToken =
    parsed.searchParams.get("access_token") ??
    parsed.searchParams.get("token") ??
    "";

  const provider = parseProvider(parsed.searchParams.get("provider"));

  if (accessToken) {
    authManager.completeSignIn(accessToken, provider);
    return;
  }

  const code = parsed.searchParams.get("code");
  if (code && provider) {
    const apiBaseUrl = connectionManager.getSnapshot().activeBaseUrl;
    await authManager.exchangeOAuthCode(code, provider, apiBaseUrl);
    return;
  }

  if (code && !provider) {
    authManager.setAuthError("OAuth callback is missing provider parameter.");
  }
}

export async function handleDeepLinkUrls(urls: string[]): Promise<void> {
  for (const url of urls) {
    await handleDeepLinkUrl(url);
  }
}
