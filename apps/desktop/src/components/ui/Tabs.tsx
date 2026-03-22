import { type KeyboardEvent, type ReactNode, useCallback, useRef, useState } from "react";

interface Tab {
  id: string;
  label: string;
  disabled?: boolean;
}

interface TabsProps {
  tabs: Tab[];
  activeTab: string;
  onTabChange: (id: string) => void;
  className?: string;
  children?: ReactNode;
}

function Tabs({ tabs, activeTab, onTabChange, className = "", children }: TabsProps) {
  const tabListRef = useRef<HTMLDivElement>(null);
  const [focusedIndex, setFocusedIndex] = useState<number>(-1);

  const enabledTabs = tabs.filter((t) => !t.disabled);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      const currentIndex = enabledTabs.findIndex((t) => t.id === activeTab);
      let nextIndex = -1;

      if (e.key === "ArrowRight") {
        e.preventDefault();
        nextIndex = (currentIndex + 1) % enabledTabs.length;
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        nextIndex = (currentIndex - 1 + enabledTabs.length) % enabledTabs.length;
      } else if (e.key === "Home") {
        e.preventDefault();
        nextIndex = 0;
      } else if (e.key === "End") {
        e.preventDefault();
        nextIndex = enabledTabs.length - 1;
      }

      if (nextIndex >= 0) {
        const tab = enabledTabs[nextIndex];
        onTabChange(tab.id);
        setFocusedIndex(tabs.indexOf(tab));
        const buttons = tabListRef.current?.querySelectorAll<HTMLButtonElement>(
          'button[role="tab"]:not(:disabled)',
        );
        buttons?.[nextIndex]?.focus();
      }
    },
    [activeTab, enabledTabs, onTabChange, tabs],
  );

  return (
    <div className={className}>
      <div
        ref={tabListRef}
        role="tablist"
        aria-orientation="horizontal"
        className="flex border-b border-[var(--border-primary)]"
        onKeyDown={handleKeyDown}
      >
        {tabs.map((tab, index) => {
          const isActive = tab.id === activeTab;
          return (
            <button
              key={tab.id}
              role="tab"
              type="button"
              id={`tab-${tab.id}`}
              aria-selected={isActive}
              aria-controls={`tabpanel-${tab.id}`}
              tabIndex={isActive ? 0 : -1}
              disabled={tab.disabled}
              onClick={() => {
                onTabChange(tab.id);
                setFocusedIndex(index);
              }}
              className={`relative px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--border-focus)] disabled:pointer-events-none disabled:opacity-50 ${
                isActive
                  ? "text-[var(--accent)] after:absolute after:bottom-0 after:left-0 after:right-0 after:h-0.5 after:bg-[var(--accent)]"
                  : "text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--surface-hover)]"
              }`}
            >
              {tab.label}
            </button>
          );
        })}
      </div>
      {children && (
        <div
          role="tabpanel"
          id={`tabpanel-${activeTab}`}
          aria-labelledby={`tab-${activeTab}`}
          tabIndex={0}
        >
          {children}
        </div>
      )}
    </div>
  );
}

Tabs.displayName = "Tabs";

export { Tabs, type TabsProps, type Tab };
