import { type SelectHTMLAttributes, forwardRef } from "react";

interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  error?: string;
  options: SelectOption[];
  placeholder?: string;
}

const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ label, error, options, placeholder, disabled, className = "", id, ...props }, ref) => {
    const selectId =
      id ?? (label ? `select-${label.toLowerCase().replace(/\s+/g, "-")}` : undefined);

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={selectId} className="text-sm font-medium text-[var(--text-primary)]">
            {label}
          </label>
        )}
        <select
          ref={ref}
          id={selectId}
          disabled={disabled}
          className={`w-full appearance-none rounded-md border bg-[var(--surface-primary)] px-3 py-2 text-sm text-[var(--text-primary)] transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--border-focus)] disabled:pointer-events-none disabled:opacity-50 ${
            error
              ? "border-[var(--error)] focus:ring-[var(--error)]"
              : "border-[var(--border-primary)]"
          } ${className}`}
          aria-invalid={error ? true : undefined}
          aria-describedby={error && selectId ? `${selectId}-error` : undefined}
          {...props}
        >
          {placeholder && (
            <option value="" disabled>
              {placeholder}
            </option>
          )}
          {options.map((option) => (
            <option key={option.value} value={option.value} disabled={option.disabled}>
              {option.label}
            </option>
          ))}
        </select>
        {error && (
          <p
            id={selectId ? `${selectId}-error` : undefined}
            className="text-xs text-[var(--error)]"
            role="alert"
          >
            {error}
          </p>
        )}
      </div>
    );
  },
);

Select.displayName = "Select";

export { Select, type SelectProps, type SelectOption };
