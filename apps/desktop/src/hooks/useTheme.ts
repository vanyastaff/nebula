import { type ThemeMode, useThemeStore } from "../lib/theme";

interface UseThemeReturn {
  theme: ThemeMode;
  setTheme: (theme: ThemeMode) => void;
}

export function useTheme(): UseThemeReturn {
  const theme = useThemeStore((s) => s.theme);
  const setTheme = useThemeStore((s) => s.setTheme);
  return { theme, setTheme };
}
