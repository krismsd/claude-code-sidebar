import type { CSSProperties } from "react";
import type { Session, Tracked, Usage } from "../lib/types";
import { usageLevel, usageNow } from "../lib/format";
import { projectStatus } from "../lib/view";

interface Props {
  projects: Tracked[];
  sessions: Session[];
  fresh: Set<string>;
  leftEdge: boolean;
  floating: boolean;
  usage: Usage | null;
  now: number;
  onExpand: () => void;
}

/** Collapsed sidebar: an expand button and one status dot per project. */
export function Strip({
  projects,
  sessions,
  fresh,
  leftEdge,
  floating,
  usage,
  now,
  onExpand,
}: Props) {
  const drag = floating ? { "data-tauri-drag-region": true } : {};
  return (
    <main className="strip" {...drag}>
      <button className="strip-expand" title="Expand the sidebar" onClick={onExpand}>
        {leftEdge ? "»" : "«"}
      </button>
      {projects.map(({ project }) => {
        const mine = sessions.filter((s) => s.project?.root === project.root);
        const { open, state, flashing } = projectStatus(mine, fresh);
        return (
          <button
            key={project.root}
            className={`strip-project ${state}${flashing ? " flash" : ""}`}
            style={{ "--project": project.color } as CSSProperties}
            title={`${project.name}: ${open} open${state === "attention" ? " · needs you" : state === "working" ? " · working" : ""}`}
            onClick={onExpand}
          >
            <span className="strip-dot" />
          </button>
        );
      })}
      {usage &&
        (() => {
          const session = usageNow(usage.five_hour, now);
          const week = usageNow(usage.seven_day, now);
          const label = (v: typeof session) =>
            !v ? "–" : v.reset ? "reset" : `${Math.round(v.percent)}%`;
          return (
            <div className="strip-usage" title={`Session ${label(session)} · week ${label(week)}`}>
              {[session, week].map((v, i) => (
                <span key={i} className={`strip-usage-bar${v?.reset ? " stale" : ""}`}>
                  <span
                    className={`usage-fill ${usageLevel(v?.percent ?? 0)}`}
                    style={{ height: `${Math.min(100, v?.percent ?? 0)}%` }}
                  />
                </span>
              ))}
            </div>
          );
        })()}
    </main>
  );
}
