import type { CSSProperties } from "react";
import type { Session, Tracked } from "../lib/types";
import { projectStatus } from "../lib/view";

interface Props {
  projects: Tracked[];
  sessions: Session[];
  fresh: Set<string>;
  leftEdge: boolean;
  floating: boolean;
  onExpand: () => void;
}

/** Collapsed sidebar: an expand button and one status dot per project. */
export function Strip({ projects, sessions, fresh, leftEdge, floating, onExpand }: Props) {
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
    </main>
  );
}
