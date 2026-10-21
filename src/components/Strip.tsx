import { useRef, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Session, Tracked, Usage } from "../lib/types";
import { usageLevel, usageNow, sessionName, STATUS_LABEL } from "../lib/format";
import { arrange, freshKeys, isOpen } from "../lib/view";

interface Props {
  projects: Tracked[];
  sessions: Session[];
  fresh: Set<string>;
  leftEdge: boolean;
  floating: boolean;
  usage: Usage | null;
  now: number;
  onExpand: () => void;
  onActivate: (s: Session) => void;
}

const ATTENTION = new Set(["needs_permission", "needs_input", "error"]);
const PEEK_OPEN_MS = 300;

const needsYou = (s: Session) =>
  ATTENTION.has(s.status) || s.alerts.some((a) => a.kind !== "compact");
const PEEK_CLOSE_MS = 80;

/** Collapsed sidebar: an expand button, one marker per session in the list (in project
 * order), and the usage bars. Hovering a marker shows its card in the peek window. */
export function Strip(props: Props) {
  const { projects, sessions, fresh, leftEdge, floating, usage, now, onExpand, onActivate } = props;
  const drag = floating ? { "data-tauri-drag-region": true } : {};
  const open = useRef(false);
  const openTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const hover = (s: Session, el: HTMLElement | null) => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
    if (!el) {
      closeTimer.current = setTimeout(() => {
        open.current = false;
        invoke("peek_hide");
      }, PEEK_CLOSE_MS);
      return;
    }
    const top = el.getBoundingClientRect().top;
    const show = () => {
      open.current = true;
      invoke("peek_show", { sessionId: s.session_id, top });
    };
    if (open.current) show();
    else openTimer.current = setTimeout(show, PEEK_OPEN_MS);
  };

  // Projects in the drawer only show sessions that need you.
  const shown = projects.flatMap(({ project, shelved, session_order }) => {
    const visible = arrange(
      sessions.filter((s) => s.project?.root === project.root),
      session_order,
    ).visible;
    return shelved ? visible.filter((s) => isOpen(s) && needsYou(s)) : visible;
  });
  const session = usageNow(usage?.five_hour ?? null, now);
  const week = usageNow(usage?.seven_day ?? null, now);
  const pct = (v: typeof session) => (!v ? "–" : v.reset ? "reset" : `${Math.round(v.percent)}%`);

  return (
    <main className="strip" {...drag}>
      <button className="strip-expand" title="Expand the sidebar" onClick={onExpand}>
        {leftEdge ? "»" : "«"}
      </button>
      {shown.map((s) => {
        const state = needsYou(s)
          ? "attention"
          : s.status === "working"
            ? "working"
            : !isOpen(s)
              ? "closed"
              : "open";
        const flashing = freshKeys(s).some((k) => fresh.has(k));
        return (
          <button
            key={s.session_id}
            className={`strip-session ${state}${flashing ? " flash" : ""}`}
            style={{ "--project": s.project?.color ?? "#888" } as CSSProperties}
            aria-label={`${sessionName(s)}: ${STATUS_LABEL[s.status]}`}
            onMouseEnter={(e) => hover(s, e.currentTarget)}
            onMouseLeave={() => hover(s, null)}
            onClick={() => {
              hover(s, null);
              onActivate(s);
            }}
          >
            <span className="strip-dot" />
          </button>
        );
      })}
      {usage && (
        <div className="strip-usage" title={`Session ${pct(session)} · week ${pct(week)}`}>
          {[session, week].map((v, i) => (
            <span key={i} className={`strip-usage-bar${v?.reset ? " stale" : ""}`}>
              <span
                className={`usage-fill ${usageLevel(v?.percent ?? 0)}`}
                style={{ height: `${Math.min(100, v?.percent ?? 0)}%` }}
              />
            </span>
          ))}
        </div>
      )}
    </main>
  );
}
