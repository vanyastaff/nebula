import { type ButtonHTMLAttributes, type ReactNode, forwardRef } from "react";

const variantClasses = {
  primary:
    "bg-[var(--accent)] text-white hover:bg-[var(--accent-hover)] hover:shadow-[var(--glow-accent)] focus-visible:ring-[var(--accent)] transition-shadow",
  secondary:
    "bg-[var(--surface-secondary)] text-[var(--text-primary)] border border-[var(--border-primary)] hover:bg-[var(--surface-hover)] hover:border-[var(--border-secondary)] focus-visible:ring-[var(--border-focus)]",
  ghost:
    "bg-transparent text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)] focus-visible:ring-[var(--border-focus)]",
  danger:
    "bg-[var(--error)] text-white hover:opacity-90 hover:shadow-[var(--glow-error)] focus-visible:ring-[var(--error)] transition-shadow",
  outline:
    "bg-transparent text-[var(--accent)] border border-[var(--border-accent)] hover:bg-[var(--accent-subtle)] focus-visible:ring-[var(--accent)]",
} as const;

const sizeClasses = {
  sm: "px-2.5 py-1 text-xs gap-1.5",
  md: "px-4 py-2 text-sm gap-2",
  lg: "px-6 py-3 text-base gap-2.5",
  "icon-sm": "p-1 text-xs",
  "icon-md": "p-2 text-sm",
  "icon-lg": "p-3 text-base",
} as const;

type ButtonVariant = keyof typeof variantClasses;
type ButtonSize = keyof typeof sizeClasses;

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  icon?: ReactNode;
}

function Spinner({ className }: { className?: string }) {
  return (
    <svg
      className={`animate-spin ${className ?? "h-4 w-4"}`}
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}

const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      variant = "primary",
      size = "md",
      loading = false,
      icon,
      disabled,
      className = "",
      children,
      ...props
    },
    ref,
  ) => {
    const isDisabled = disabled || loading;

    return (
      <button
        ref={ref}
        disabled={isDisabled}
        className={`inline-flex items-center justify-center rounded-md font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--surface-primary)] disabled:pointer-events-none disabled:opacity-50 ${variantClasses[variant]} ${sizeClasses[size]} ${className}`}
        {...props}
      >
        {loading ? <Spinner /> : icon}
        {children}
      </button>
    );
  },
);

Button.displayName = "Button";

export { Button, type ButtonProps, type ButtonVariant, type ButtonSize };
