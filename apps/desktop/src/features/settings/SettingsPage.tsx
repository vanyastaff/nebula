import {
  CheckCircle2,
  ExternalLink,
  Github,
  Globe,
  Info,
  Laptop,
  LogOut,
  Moon,
  Sun,
  User,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";

import { Avatar } from "../../components/ui/Avatar";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { type Locale, type Theme, useSettingsStore } from "../../stores/settingsStore";
import { type UserProfile, useAuthStore } from "../auth/store";

const APP_VERSION = "0.1.0-alpha";
const BUILD_DATE = "March 2026";

type Section = "appearance" | "language" | "account" | "about";

interface NavItem {
  id: Section;
  labelKey: string;
  icon: ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  { id: "appearance", labelKey: "settings.navAppearance", icon: <Sun size={16} /> },
  { id: "language", labelKey: "settings.navLanguage", icon: <Globe size={16} /> },
  { id: "account", labelKey: "settings.navAccount", icon: <User size={16} /> },
  { id: "about", labelKey: "settings.navAbout", icon: <Info size={16} /> },
];

const LANGUAGES: Array<{ value: Locale; native: string; english: string; flag: string }> = [
  { value: "en", native: "English", english: "English", flag: "🇺🇸" },
  { value: "ru", native: "Русский", english: "Russian", flag: "🇷🇺" },
];

function SectionHeading({ children }: { children: ReactNode }) {
  return (
    <h2 className="mb-5 text-[11px] font-semibold uppercase tracking-widest text-[var(--text-tertiary)]">
      {children}
    </h2>
  );
}

function SettingRow({
  label,
  description,
  danger = false,
  children,
}: {
  label: string;
  description?: string;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-[var(--border-primary)] py-3.5 last:border-0">
      <div className="min-w-0 flex-1">
        <p
          className={`text-sm font-medium ${
            danger ? "text-[var(--error)]" : "text-[var(--text-primary)]"
          }`}
        >
          {label}
        </p>
        {description && (
          <p className="mt-0.5 text-xs leading-relaxed text-[var(--text-tertiary)]">
            {description}
          </p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

interface ThemeCardProps {
  value: Theme;
  current: Theme;
  label: string;
  onClick: (value: Theme) => void;
}

function ThemeCard({ value, current, label, onClick }: ThemeCardProps) {
  const isSelected = current === value;
  const isDark = value === "dark";
  const isSystem = value === "system";

  return (
    <button
      type="button"
      onClick={() => onClick(value)}
      className={`relative flex cursor-pointer flex-col items-center gap-2.5 rounded-xl border p-3 transition-all focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
        isSelected
          ? "border-[var(--accent)] bg-[var(--accent-subtle)] shadow-[var(--glow-accent)]"
          : "border-[var(--border-primary)] bg-[var(--surface-primary)] hover:border-[var(--border-secondary)] hover:bg-[var(--surface-hover)]"
      }`}
    >
      {/* Mini theme preview */}
      <div
        className={`relative h-14 w-full overflow-hidden rounded-lg border border-[var(--border-primary)] ${
          isDark ? "bg-[#080e1e]" : "bg-white"
        }`}
        style={
          isSystem ? { background: "linear-gradient(135deg, #ffffff 50%, #080e1e 50%)" } : undefined
        }
      >
        {!isSystem && (
          <div
            className={`absolute bottom-0 left-0 top-0 w-5 ${
              isDark ? "bg-[#050912]" : "bg-gray-100"
            }`}
          />
        )}
        <div className="absolute left-1 top-1 flex gap-0.5">
          <div className={`h-1 w-1 rounded-full ${isDark ? "bg-[#1e2d4a]" : "bg-gray-300"}`} />
          <div className={`h-1 w-1 rounded-full ${isDark ? "bg-[#1e2d4a]" : "bg-gray-300"}`} />
          <div className={`h-1 w-1 rounded-full ${isDark ? "bg-[#1e2d4a]" : "bg-gray-300"}`} />
        </div>
        <div className="absolute left-7 right-2 top-4 space-y-1">
          <div className={`h-1 w-full rounded ${isDark ? "bg-[#1e2d4a]" : "bg-gray-200"}`} />
          <div className={`h-1 w-3/4 rounded ${isDark ? "bg-[#141f35]" : "bg-gray-100"}`} />
        </div>
        <div
          className="absolute bottom-1.5 right-2 h-2 w-5 rounded"
          style={{ background: isDark ? "#a78bfa" : "#7c3aed" }}
        />
        {isSystem && (
          <div className="absolute inset-0 flex items-center justify-center">
            <Laptop size={14} className="text-gray-400" />
          </div>
        )}
      </div>

      {/* Label row */}
      <div className="flex items-center gap-1.5">
        {value === "light" && (
          <Sun
            size={11}
            className={isSelected ? "text-[var(--accent)]" : "text-[var(--text-tertiary)]"}
          />
        )}
        {value === "dark" && (
          <Moon
            size={11}
            className={isSelected ? "text-[var(--accent)]" : "text-[var(--text-tertiary)]"}
          />
        )}
        {value === "system" && (
          <Laptop
            size={11}
            className={isSelected ? "text-[var(--accent)]" : "text-[var(--text-tertiary)]"}
          />
        )}
        <span
          className={`text-xs font-medium ${
            isSelected ? "text-[var(--accent)]" : "text-[var(--text-secondary)]"
          }`}
        >
          {label}
        </span>
      </div>

      {isSelected && (
        <div className="absolute right-2 top-2">
          <CheckCircle2 size={13} className="text-[var(--accent)]" />
        </div>
      )}
    </button>
  );
}

function AppearanceSection({
  theme,
  onThemeChange,
}: {
  theme: Theme;
  onThemeChange: (t: Theme) => void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <SectionHeading>{t("settings.navAppearance")}</SectionHeading>
      <p className="mb-1 text-sm font-medium text-[var(--text-primary)]">{t("settings.theme")}</p>
      <p className="mb-4 text-xs text-[var(--text-tertiary)]">{t("settings.themeDesc")}</p>
      <div className="grid grid-cols-3 gap-3">
        <ThemeCard
          value="light"
          current={theme}
          label={t("settings.themeLight")}
          onClick={onThemeChange}
        />
        <ThemeCard
          value="dark"
          current={theme}
          label={t("settings.themeDark")}
          onClick={onThemeChange}
        />
        <ThemeCard
          value="system"
          current={theme}
          label={t("settings.themeSystem")}
          onClick={onThemeChange}
        />
      </div>
    </div>
  );
}

function LanguageSection({
  locale,
  onLocaleChange,
}: {
  locale: Locale;
  onLocaleChange: (l: Locale) => void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <SectionHeading>{t("settings.navLanguage")}</SectionHeading>
      <p className="mb-4 text-xs text-[var(--text-tertiary)]">{t("settings.languageDesc")}</p>
      <div className="overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
        {LANGUAGES.map((lang) => (
          <button
            key={lang.value}
            type="button"
            onClick={() => onLocaleChange(lang.value)}
            className="flex w-full cursor-pointer items-center gap-3 border-b border-[var(--border-primary)] px-4 py-3 text-left transition-colors last:border-0 hover:bg-[var(--surface-hover)] focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--accent)]"
          >
            <span className="text-xl leading-none">{lang.flag}</span>
            <div className="flex-1">
              <p className="text-sm font-medium text-[var(--text-primary)]">{lang.native}</p>
              <p className="text-xs text-[var(--text-tertiary)]">{lang.english}</p>
            </div>
            {locale === lang.value && (
              <CheckCircle2 size={16} className="shrink-0 text-[var(--accent)]" />
            )}
          </button>
        ))}
      </div>
    </div>
  );
}

function AccountSection({
  user,
  onSignOut,
}: {
  user: UserProfile | undefined;
  onSignOut: () => Promise<void>;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <SectionHeading>{t("settings.navAccount")}</SectionHeading>
      {user && (
        <div className="mb-4 overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
          <div className="flex items-center gap-4 border-b border-[var(--border-primary)] p-4">
            <Avatar name={user.name ?? user.login} src={user.avatarUrl ?? undefined} size="lg" />
            <div className="min-w-0 flex-1">
              <p className="truncate font-semibold text-[var(--text-primary)]">
                {user.name ?? user.login}
              </p>
              {user.email && (
                <p className="truncate text-sm text-[var(--text-secondary)]">{user.email}</p>
              )}
              <p className="mt-0.5 text-xs text-[var(--text-tertiary)]">@{user.login}</p>
            </div>
            <Badge variant="success" size="sm">
              {t("settings.connected")}
            </Badge>
          </div>
          <SettingRow label={t("settings.provider")} description={t("settings.connectedVia")}>
            <div className="flex items-center gap-1.5 text-sm text-[var(--text-secondary)]">
              <Github size={14} />
              <span>GitHub</span>
            </div>
          </SettingRow>
        </div>
      )}

      <div
        className="overflow-hidden rounded-xl"
        style={{
          border: "1px solid rgba(239,68,68,0.3)",
          background: "var(--error-subtle)",
        }}
      >
        <div className="border-b px-4 py-2" style={{ borderColor: "rgba(239,68,68,0.2)" }}>
          <p className="text-[11px] font-semibold uppercase tracking-widest text-[var(--error)]">
            {t("settings.dangerZone")}
          </p>
        </div>
        <div className="flex items-center justify-between gap-6 p-4">
          <div>
            <p className="text-sm font-medium text-[var(--text-primary)]">{t("auth.logout")}</p>
            <p className="mt-0.5 text-xs text-[var(--text-tertiary)]">
              {t("settings.signOutDesc")}
            </p>
          </div>
          <Button variant="danger" size="sm" icon={<LogOut size={13} />} onClick={onSignOut}>
            {t("auth.logout")}
          </Button>
        </div>
      </div>
    </div>
  );
}

function AboutSection() {
  const { t } = useTranslation();
  return (
    <div>
      <SectionHeading>{t("settings.navAbout")}</SectionHeading>
      <div className="mb-4 overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
        <div className="flex items-center gap-4 border-b border-[var(--border-primary)] p-4">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-[var(--accent)] to-violet-700 shadow-[0_0_16px_var(--accent-glow)]">
            <span className="text-xl font-bold leading-none text-white">N</span>
          </div>
          <div className="flex-1">
            <p className="font-semibold text-[var(--text-primary)]">Nebula</p>
            <p className="text-sm text-[var(--text-secondary)]">Workflow automation engine</p>
          </div>
          <Badge variant="accent" size="sm">
            Alpha
          </Badge>
        </div>
        <SettingRow label={t("settings.version")}>
          <span className="font-mono text-sm text-[var(--text-secondary)]">{APP_VERSION}</span>
        </SettingRow>
        <SettingRow label={t("settings.build")}>
          <span className="text-sm text-[var(--text-secondary)]">{BUILD_DATE}</span>
        </SettingRow>
      </div>

      <div className="overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)]">
        <SettingRow
          label={t("settings.viewDocs")}
          description="Guides, API reference, and examples"
        >
          <Button variant="ghost" size="sm" icon={<ExternalLink size={13} />}>
            Open
          </Button>
        </SettingRow>
        <SettingRow label={t("settings.reportBug")} description="Help us improve Nebula">
          <Button variant="ghost" size="sm" icon={<ExternalLink size={13} />}>
            GitHub
          </Button>
        </SettingRow>
        <SettingRow label={t("settings.releaseNotes")} description="What's new in this version">
          <Button variant="ghost" size="sm" icon={<ExternalLink size={13} />}>
            View
          </Button>
        </SettingRow>
      </div>
    </div>
  );
}

export function SettingsPage() {
  const [activeSection, setActiveSection] = useState<Section>("appearance");
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

  return (
    <div className="flex h-full bg-[var(--bg-primary)]">
      <nav
        className="flex w-52 shrink-0 flex-col gap-0.5 border-r border-[var(--border-primary)] p-4"
        aria-label={t("settings.title")}
      >
        <p className="mb-3 px-3 text-[11px] font-semibold uppercase tracking-widest text-[var(--text-tertiary)]">
          {t("settings.title")}
        </p>
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => setActiveSection(item.id)}
            aria-current={activeSection === item.id ? "page" : undefined}
            className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
              activeSection === item.id
                ? "bg-[var(--accent-subtle)] text-[var(--accent)]"
                : "text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
            }`}
          >
            <span
              className={`shrink-0 ${
                activeSection === item.id ? "text-[var(--accent)]" : "text-[var(--text-tertiary)]"
              }`}
            >
              {item.icon}
            </span>
            {t(item.labelKey)}
          </button>
        ))}
      </nav>

      <main className="flex-1 overflow-y-auto p-8">
        <div className="max-w-lg">
          {activeSection === "appearance" && (
            <AppearanceSection theme={theme} onThemeChange={handleThemeChange} />
          )}
          {activeSection === "language" && (
            <LanguageSection locale={locale} onLocaleChange={handleLanguageChange} />
          )}
          {activeSection === "account" && <AccountSection user={user} onSignOut={logout} />}
          {activeSection === "about" && <AboutSection />}
        </div>
      </main>
    </div>
  );
}
