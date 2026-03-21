import { type HTMLAttributes, forwardRef, type ReactNode } from "react";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  header?: ReactNode;
  footer?: ReactNode;
}

const Card = forwardRef<HTMLDivElement, CardProps>(
  ({ header, footer, className = "", children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={`rounded-lg border border-[var(--border-primary)] bg-[var(--surface-primary)] ${className}`}
        {...props}
      >
        {header && (
          <div className="border-b border-[var(--border-primary)] px-4 py-3 text-sm font-medium text-[var(--text-primary)]">
            {header}
          </div>
        )}
        <div className="px-4 py-4">{children}</div>
        {footer && (
          <div className="border-t border-[var(--border-primary)] px-4 py-3 text-sm text-[var(--text-secondary)]">
            {footer}
          </div>
        )}
      </div>
    );
  },
);

Card.displayName = "Card";

export { Card, type CardProps };
