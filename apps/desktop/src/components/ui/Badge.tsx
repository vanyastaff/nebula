import { type HTMLAttributes, forwardRef } from "react";

const variantClasses = {
  success:
    "bg-[var(--success)]/15 text-[var(--success)] border-[var(--success)]/25",
  warning:
    "bg-[var(--warning)]/15 text-[var(--warning)] border-[var(--warning)]/25",
  error:
    "bg-[var(--error)]/15 text-[var(--error)] border-[var(--error)]/25",
  info:
    "bg-[var(--info)]/15 text-[var(--info)] border-[var(--info)]/25",
  neutral:
    "bg-[var(--surface-secondary)] text-[var(--text-secondary)] border-[var(--border-primary)]",
  accent:
    "bg-[var(--accent)]/15 text-[var(--accent)] border-[var(--accent)]/25",
} as const;

const sizeClasses = {
  sm: "px-1.5 py-0.5 text-[10px]",
  md: "px-2 py-0.5 text-xs",
} as const;

type BadgeVariant = keyof typeof variantClasses;
type BadgeSize = keyof typeof sizeClasses;

interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant;
  size?: BadgeSize;
}

const Badge = forwardRef<HTMLSpanElement, BadgeProps>(
  ({ variant = "neutral", size = "md", className = "", children, ...props }, ref) => {
    return (
      <span
        ref={ref}
        className={`inline-flex items-center rounded-full border font-medium leading-none ${variantClasses[variant]} ${sizeClasses[size]} ${className}`}
        {...props}
      >
        {children}
      </span>
    );
  },
);

Badge.displayName = "Badge";

export { Badge, type BadgeProps, type BadgeVariant, type BadgeSize };
