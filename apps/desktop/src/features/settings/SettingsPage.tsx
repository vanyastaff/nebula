import { useTranslation } from "react-i18next";
import { Card } from "../../components/ui/Card";
import { Button } from "../../components/ui/Button";
import { Avatar } from "../../components/ui/Avatar";
import { useSettingsStore, type Theme, type Locale } from "../../stores/settingsStore";
import { useAuthStore } from "../auth/store";

export function SettingsPage() {
  const { t, i18n } = useTranslation();
  const { theme, locale, setTheme, setLocale } = useSettingsStore();
  const { user, logout } = useAuthStore();

  const handleThemeChange = async (newTheme: Theme) => {
    await setTheme(newTheme);
  };

  const handleLanguageChange = async (newLocale: Locale) => {
    await setLocale(newLocale);
    await i18n.changeLanguage(newLocale);
  };

  const handleSignOut = async () => {
    await logout();
  };

  return (
    <div className="p-6">
      <h1 className="mb-6 text-2xl font-bold text-[var(--text-primary)]">
        {t("settings.title")}
      </h1>

      <div className="flex flex-col gap-4">
        {/* Appearance Section */}
        <Card header={t("settings.appearance")}>
          <div className="space-y-4">
            {/* Theme */}
            <div>
              <label className="mb-2 block text-sm font-medium text-[var(--text-primary)]">
                {t("settings.theme")}
              </label>
              <div className="flex gap-2">
                <Button
                  variant={theme === "light" ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => handleThemeChange("light")}
                >
                  {t("settings.themeLight")}
                </Button>
                <Button
                  variant={theme === "dark" ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => handleThemeChange("dark")}
                >
                  {t("settings.themeDark")}
                </Button>
                <Button
                  variant={theme === "system" ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => handleThemeChange("system")}
                >
                  {t("settings.themeSystem")}
                </Button>
              </div>
            </div>
          </div>
        </Card>

        {/* Language Section */}
        <Card header={t("settings.language")}>
          <div>
            <label className="mb-2 block text-sm font-medium text-[var(--text-primary)]">
              {t("settings.selectLanguage")}
            </label>
            <div className="flex gap-2">
              <Button
                variant={locale === "en" ? "primary" : "secondary"}
                size="sm"
                onClick={() => handleLanguageChange("en")}
              >
                English
              </Button>
              <Button
                variant={locale === "ru" ? "primary" : "secondary"}
                size="sm"
                onClick={() => handleLanguageChange("ru")}
              >
                Русский
              </Button>
            </div>
          </div>
        </Card>

        {/* Account Section */}
        <Card header={t("settings.account")}>
          <div className="space-y-4">
            {user && (
              <div className="flex items-center gap-3">
                <Avatar
                  name={user.name ?? user.email ?? "User"}
                  src={user.avatarUrl ?? undefined}
                  size="lg"
                />
                <div className="flex-1">
                  <p className="font-medium text-[var(--text-primary)]">
                    {user.name ?? user.email}
                  </p>
                  {user.name && user.email && (
                    <p className="text-sm text-[var(--text-secondary)]">{user.email}</p>
                  )}
                </div>
              </div>
            )}
            <div>
              <Button variant="danger" size="md" onClick={handleSignOut}>
                {t("auth.logout")}
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
