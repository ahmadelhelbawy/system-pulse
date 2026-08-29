import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  /** Shown in the fallback UI, e.g. "Overview" — helps identify which
   * panel crashed without taking down the rest of the app. */
  label: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Catches a render error in one panel so it can't white-screen the whole
 * app (1.0 had no error boundary anywhere) — the other tabs, the toolbar,
 * and the status bar stay usable.
 */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[${this.props.label}] render error:`, error, info.componentStack);
  }

  private reset = () => this.setState({ error: null });

  render() {
    if (this.state.error) {
      return (
        <div className="empty-state" role="alert">
          <div className="empty-state__title">{this.props.label} hit an error</div>
          <div className="empty-state__detail">{this.state.error.message}</div>
          <button className="button" onClick={this.reset}>
            Try again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
