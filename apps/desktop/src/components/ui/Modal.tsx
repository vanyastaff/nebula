import { type DialogHTMLAttributes, type ReactNode, forwardRef, useEffect, useRef, useCallback } from "react";

const sizeClasses = {
  sm: "max-w-sm",
  md: "max-w-lg",
  lg: "max-w-2xl",
  full: "max-w-[90vw]",
} as const;

type ModalSize = keyof typeof sizeClasses;

interface ModalProps extends Omit<DialogHTMLAttributes<HTMLDialogElement>, "open"> {
  open: boolean;
  onClose: () => void;
  size?: ModalSize;
  title?: string;
  children: ReactNode;
}

const Modal = forwardRef<HTMLDialogElement, ModalProps>(
  ({ open, onClose, size = "md", title, children, className = "", ...props }, ref) => {
    const internalRef = useRef<HTMLDialogElement>(null);
    const dialogRef = (ref as React.RefObject<HTMLDialogElement>) ?? internalRef;

    useEffect(() => {
      const dialog = dialogRef.current;
      if (!dialog) return;

      if (open && !dialog.open) {
        dialog.showModal();
      } else if (!open && dialog.open) {
        dialog.close();
      }
    }, [open, dialogRef]);

    const handleCancel = useCallback(
      (e: React.SyntheticEvent) => {
        e.preventDefault();
        onClose();
      },
      [onClose],
    );

    const handleBackdropClick = useCallback(
      (e: React.MouseEvent<HTMLDialogElement>) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      },
      [onClose],
    );

    return (
      <dialog
        ref={dialogRef}
        onCancel={handleCancel}
        onClick={handleBackdropClick}
        className={`${sizeClasses[size]} w-full rounded-xl border border-[var(--border-primary)] bg-[var(--surface-primary)] p-0 text-[var(--text-primary)] shadow-xl backdrop:bg-black/50 backdrop:backdrop-blur-sm open:animate-in open:fade-in-0 open:slide-in-from-bottom-4 ${className}`}
        {...props}
      >
        <div className="flex flex-col">
          {title && (
            <div className="flex items-center justify-between border-b border-[var(--border-primary)] px-6 py-4">
              <h2 className="text-lg font-semibold text-[var(--text-primary)]">{title}</h2>
              <button
                type="button"
                onClick={onClose}
                className="rounded-md p-1 text-[var(--text-tertiary)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--border-focus)]"
                aria-label="Close"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="M18 6 6 18" />
                  <path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
          )}
          <div className="px-6 py-4">{children}</div>
        </div>
      </dialog>
    );
  },
);

Modal.displayName = "Modal";

export { Modal, type ModalProps, type ModalSize };
