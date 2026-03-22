import { type HTMLAttributes, type ReactNode, forwardRef } from "react";

const positionClasses = {
  top: "bottom-full left-1/2 -translate-x-1/2 mb-2",
  bottom: "top-full left-1/2 -translate-x-1/2 mt-2",
  left: "right-full top-1/2 -translate-y-1/2 mr-2",
  right: "left-full top-1/2 -translate-y-1/2 ml-2",
} as const;

type TooltipPosition = keyof typeof positionClasses;

interface TooltipProps extends HTMLAttributes<HTMLDivElement> {
  content: string;
  position?: TooltipPosition;
  children: ReactNode;
}

const Tooltip = forwardRef<HTMLDivElement, TooltipProps>(
  ({ content, position = "top", children, className = "", ...props }, ref) => {
    return (
      <div ref={ref} className={`group relative inline-flex ${className}`} {...props}>
        {children}
        <span
          role="tooltip"
          className={`pointer-events-none absolute z-50 whitespace-nowrap rounded-md bg-[var(--surface-tertiary)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-primary)] opacity-0 shadow-lg transition-opacity group-hover:opacity-100 ${positionClasses[position]}`}
        >
          {content}
        </span>
      </div>
    );
  },
);

Tooltip.displayName = "Tooltip";

export { Tooltip, type TooltipProps, type TooltipPosition };
