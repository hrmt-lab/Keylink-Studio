import { AlertCircle, Check } from "lucide-react";
import type { ReactNode } from "react";

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between">
      <div>
        <h1 className="text-xl font-medium text-ink">{title}</h1>
        <p className="mt-0.5 text-sm text-muted">{description}</p>
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  );
}

export function SpinnerIcon({ inverse = false }: { inverse?: boolean }) {
  return (
    <div
      className={`h-4 w-4 animate-spin rounded-full border-2 ${
        inverse ? "border-white/30 border-t-white" : "border-border border-t-accent"
      }`}
    />
  );
}

export function PrimaryButton({
  children,
  onClick,
  disabled,
  loading,
  icon,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  loading?: boolean;
  icon?: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      className="btn-neu flex items-center gap-2 rounded-full px-5 py-2.5 text-sm font-medium text-ink disabled:opacity-50"
    >
      {loading ? <SpinnerIcon /> : icon}
      {children}
    </button>
  );
}

export function SecondaryButton({
  children,
  onClick,
  disabled,
  loading,
  icon,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  loading?: boolean;
  icon?: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      className="row-lift flex items-center gap-2 rounded-full border border-border bg-surface px-4 py-2.5 text-sm font-medium text-muted hover:text-ink disabled:cursor-not-allowed disabled:opacity-45"
    >
      {loading ? <SpinnerIcon /> : icon}
      {children}
    </button>
  );
}

export function ErrorNotice({
  message,
  details,
}: {
  message: string;
  details?: string | null;
}) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
      <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
      <div className="min-w-0">
        <div>{message}</div>
        {details && <div className="mt-0.5 break-words text-red-600">{details}</div>}
      </div>
    </div>
  );
}

export function Notice({
  tone = "info",
  children,
}: {
  tone?: "info" | "warn" | "error";
  children: ReactNode;
}) {
  const color =
    tone === "error"
      ? "bg-red-50 text-red-700 ring-red-200"
      : tone === "warn"
        ? "bg-amber-50 text-amber-800 ring-amber-200"
        : "bg-plate text-ink ring-border";
  return (
    <div className={`flex items-start gap-2.5 rounded-lg px-4 py-3 text-sm ring-1 ${color}`}>
      <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
      <span>{children}</span>
    </div>
  );
}

/** Transient saved feedback: check mark only, fades in and out on its own. */
export function SavedIndicator({ label }: { label: string }) {
  return (
    <span
      role="status"
      aria-label={label}
      className="animate-saved-pop flex items-center text-accent-deep"
    >
      <Check size={15} className="flex-shrink-0" />
    </span>
  );
}

export function SectionCard({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="overflow-hidden rounded-card bg-surface">
      {title && (
        <div className="border-b border-background px-5 py-3">
          <h2 className="text-sm font-medium text-ink">{title}</h2>
        </div>
      )}
      <div className="divide-y divide-background">{children}</div>
    </div>
  );
}

export function SettingRow({
  label,
  description,
  children,
  compact = false,
  align = "center",
}: {
  label: string;
  description: string;
  children: ReactNode;
  compact?: boolean;
  align?: "center" | "start";
}) {
  return (
    <div
      className={`flex justify-between gap-4 px-5 ${
        compact ? "py-3" : "py-4"
      } ${align === "start" ? "items-start" : "items-center"}`}
    >
      <div className="min-w-0">
        <div className={`${compact ? "text-xs" : "text-sm"} font-medium text-ink`}>
          {label}
        </div>
        <div className="mt-0.5 text-xs text-muted">{description}</div>
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}
