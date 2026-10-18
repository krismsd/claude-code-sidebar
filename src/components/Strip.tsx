import type { CSSProperties } from "react";
import type { Session, Tracked, Usage } from "../lib/types";
import { usageLevel } from "../lib/format";
import { projectStatus } from "../lib/view";

interface Props {
  projects: Tracked[];
  sessions: Session[];
  fresh: Set<string>;
  leftEdge: boolean;
  floating: boolean;
  usage: Usage | null;
  onExpand: () => void;
}

/** Collapsed sidebar: an expand button and one status dot per project. */
export function Strip({ projects, sessions, fresh, leftEdge, floating, usage, onExpand }: Props) {
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
      {usage && (
        <div
          className="strip-usage"
          title={`Session ${Math.round(usage.five_hour?.percent ?? 0)}% · week ${Math.round(usage.seven_day?.percent ?? 0)}%`}
        >
          {[usage.five_hour, usage.seven_day].map((w, i) => (
            <span key={i} className="strip-usage-bar">
              <span
                className={`usage-fill ${usageLevel(w?.percent ?? 0)}`}
                style={{ height: `${Math.min(100, w?.percent ?? 0)}%` }}
              />
            </span>
          ))}
        </div>
      )}
    </main>
  );
}
