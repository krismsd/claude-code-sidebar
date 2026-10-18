import type { Usage, UsageWindow } from "../lib/types";
import { elapsed, resetTime, usageLevel, usageNow } from "../lib/format";

interface Props {
  usage: Usage | null;
  now: number;
}

function Row({ label, w, now }: { label: string; w: UsageWindow | null; now: number }) {
  const v = usageNow(w, now);
  if (!w || !v) return null;
  const done = v.reset;
  const pct = Math.round(w.percent);
  return (
    <div className={`usage-row${done ? " stale" : ""}`}>
      <span className="usage-label">{label}</span>
      <span className="usage-bar">
        <span
          className={`usage-fill ${usageLevel(v.percent)}`}
          style={{ width: `${Math.min(100, v.percent)}%` }}
        />
      </span>
      <span className="usage-pct">{done ? "–" : `${pct}%`}</span>
      <span className="usage-reset">
        {done ? "reset" : w.resets_at !== null ? resetTime(w.resets_at, now) : ""}
      </span>
    </div>
  );
}

/** Subscription usage at the foot of the sidebar (the numbers /usage shows). */
export function UsageFooter({ usage, now }: Props) {
  if (!usage)
    return (
      <footer className="usage">
        <span className="muted small">Usage limits: checking…</span>
      </footer>
    );
  return (
    <footer
      className="usage"
      title={`Usage limits, updated ${elapsed(usage.updated_at, now)} ago (they update after each turn)`}
    >
      <Row label="Session" w={usage.five_hour} now={now} />
      <Row label="Week" w={usage.seven_day} now={now} />
      {usage.status === "rejected" && <div className="usage-warn">Limit reached</div>}
      {usage.using_overage && <div className="usage-warn">Using extra usage</div>}
    </footer>
  );
}
