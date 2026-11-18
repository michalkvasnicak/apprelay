import type { ReactNode } from "react";

export const inputClass =
  "mt-2 block w-full rounded-xl border border-text/15 bg-background/70 px-3.5 py-3 text-sm text-text transition placeholder:text-muted/60 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/30 disabled:opacity-50";
export const buttonClass =
  "inline-flex cursor-pointer items-center justify-center gap-2 rounded-xl bg-primary px-5 py-3 text-sm font-semibold text-background transition hover:bg-primary/90 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100";
export const secondaryClass =
  "inline-flex cursor-pointer items-center justify-center gap-2 rounded-xl border border-text/15 px-4 py-2.5 text-sm font-medium text-text transition hover:bg-text/5 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100";
export const linkClass = "text-primary underline underline-offset-4 hover:text-primary/80";

export function Spinner({ className = "size-4" }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      className={`${className} shrink-0 animate-spin`}
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

/** Button label that swaps in a spinner and a pending label while work is running. */
export function Pending({
  active,
  label,
  children,
}: {
  active: boolean;
  label: string;
  children: ReactNode;
}) {
  return active ? (
    <>
      <Spinner />
      {label}
    </>
  ) : (
    children
  );
}

export function Alert({
  error,
  onDismiss,
  action,
}: {
  error: Error | string | null | undefined;
  onDismiss?: () => void;
  action?: ReactNode;
}) {
  if (!error) {
    return null;
  }
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-error/30 bg-error/10 px-4 py-3 text-sm leading-6 text-error"
    >
      <span className="flex-1">{typeof error === "string" ? error : error.message}</span>
      {action}
      {onDismiss && (
        <button
          type="button"
          aria-label="Dismiss error"
          onClick={onDismiss}
          className="-my-0.5 cursor-pointer rounded-md px-1.5 text-lg leading-6 hover:bg-error/15"
        >
          ×
        </button>
      )}
    </div>
  );
}

export const ActionError = Alert;

export function Status({ children, busy = true }: { children: ReactNode; busy?: boolean }) {
  return (
    <p role="status" className="flex items-center gap-2.5 text-sm text-muted">
      {busy && <Spinner className="size-4 text-primary" />}
      <span>{children}</span>
    </p>
  );
}

const steps = ["Apple", "iPhone", "Sign"] as const;

export function Steps({ current }: { current: 0 | 1 | 2 }) {
  return (
    <ol aria-label="Progress" className="mb-6 flex items-center gap-2 text-xs font-medium">
      {steps.map((name, index) => {
        const done = index < current;
        const active = index === current;
        let badgeClass = "border-text/20 text-muted";
        if (done) {
          badgeClass = "border-primary bg-primary text-background";
        } else if (active) {
          badgeClass = "border-primary text-primary";
        }
        return (
          <li
            key={name}
            aria-current={active ? "step" : undefined}
            className="flex flex-1 items-center gap-2"
          >
            <span
              className={`grid size-6 shrink-0 place-items-center rounded-full border text-[11px] transition ${badgeClass}`}
            >
              {done ? "✓" : index + 1}
            </span>
            <span className={active ? "text-text" : "text-muted"}>{name}</span>
            {index < steps.length - 1 && (
              <span className={`h-px flex-1 ${done ? "bg-primary" : "bg-text/15"}`} />
            )}
          </li>
        );
      })}
    </ol>
  );
}
