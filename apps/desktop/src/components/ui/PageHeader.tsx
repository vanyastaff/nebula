import { type ReactNode } from "react";

interface PageHeaderProps {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  className?: string;
}

function PageHeader({ title, subtitle, actions, className = "" }: PageHeaderProps) {
  return (
    <div className={`flex items-start justify-between gap-4 ${className}`}>
      <div className="min-w-0">
        <h1 className="text-2xl font-semibold text-[var(--text-primary)] truncate">{title}</h1>
        {subtitle && (
          <p className="mt-1 text-sm text-[var(--text-secondary)]">{subtitle}</p>
        )}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}

export { PageHeader, type PageHeaderProps };
