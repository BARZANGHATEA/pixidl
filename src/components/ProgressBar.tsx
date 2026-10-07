// Renders only real values from the backend. When the total size is unknown
// (or the engine is still preparing) a static striped bar is shown — never a
// fake, self-advancing progress animation.
export type BarTone = "active" | "done" | "error" | "paused" | "indeterminate";

export function ProgressBar({ value, tone, label }: { value: number | null; tone: BarTone; label: string }) {
  const width = tone === "indeterminate" || value === null ? 100 : value;
  return (
    <div
      className={`bar ${tone === "active" ? "" : tone}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(value)}
    >
      <i style={{ width: `${width}%` }} />
    </div>
  );
}
