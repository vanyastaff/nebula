import { type HTMLAttributes, type ReactNode, forwardRef } from "react";

const cardVariantClasses = {
  default: "border border-[var(--border-primary)] bg-[var(--surface-primary)]",
  elevated:
    "border border-[var(--border-primary)] bg-[var(--surface-primary)] shadow-[var(--shadow-md)]",
  glass:
    "border border-[var(--glass-border)] bg-[var(--glass-bg)] backdrop-blur-[var(--glass-blur)] shadow-[var(--glass-shadow)]",
  feature:
    "border border-[var(--border-accent)] bg-[var(--accent-subtle)] hover:shadow-[var(--glow-accent)] transition-shadow",
} as const;

const cardPaddingClasses = {
  sm: "px-3 py-2",
  md: "px-4 py-4",
  lg: "px-6 py-6",
} as const;

type CardVariant = keyof typeof cardVariantClasses;
type CardPadding = keyof typeof cardPaddingClasses;

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  header?: ReactNode;
  footer?: ReactNode;
  variant?: CardVariant;
  padding?: CardPadding;
}

const Card = forwardRef<HTMLDivElement, CardProps>(
  (
    { header, footer, variant = "default", padding = "md", className = "", children, ...props },
    ref,
  ) => {
    return (
      <div
        ref={ref}
        className={`rounded-lg ${cardVariantClasses[variant]} ${className}`}
        {...props}
      >
        {header && (
          <div className="border-b border-[var(--border-primary)] px-4 py-3 text-sm font-medium text-[var(--text-primary)]">
            {header}
          </div>
        )}
        <div className={cardPaddingClasses[padding]}>{children}</div>
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

export { Card, type CardProps, type CardVariant, type CardPadding };
