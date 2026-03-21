import { Component, type ErrorInfo, type ReactNode } from "react";

interface ErrorBoundaryProps {
  children: ReactNode;
  fallback?: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    // TODO: integrate with telemetry/logging service
    void { error, errorInfo };
  }

  private handleRetry = (): void => {
    this.setState({ hasError: false, error: null });
  };

  render(): ReactNode {
    if (!this.state.hasError) {
      return this.props.children;
    }

    if (this.props.fallback) {
      return this.props.fallback;
    }

    return (
      <div className="flex min-h-[200px] flex-col items-center justify-center gap-4 p-8">
        <div className="text-center">
          <h2 className="mb-2 text-lg font-semibold text-[var(--text-primary)]">
            Something went wrong
          </h2>
          <p className="max-w-md text-sm text-[var(--text-secondary)]">
            {this.state.error?.message ?? "An unexpected error occurred."}
          </p>
        </div>
        <button
          type="button"
          onClick={this.handleRetry}
          className="rounded-md bg-[var(--accent)] px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-[var(--accent-hover)] focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:ring-offset-2"
        >
          Try Again
        </button>
      </div>
    );
  }
}
