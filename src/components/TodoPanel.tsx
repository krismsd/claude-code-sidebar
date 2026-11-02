import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Todo, TodoState } from "../lib/types";
import { openTodos, todoParts } from "../lib/view";

interface Props {
  root: string;
  todos: Todo[];
  onClose: () => void;
  onMessage: (msg: string) => void;
}

/** A project's running to-do list (SPEC §3.19): noted here while sessions work, pasted into a
 * prompt (Copy), or read and ticked off by sessions through the sidebar's MCP server. */
export function TodoPanel({ root, todos, onClose, onMessage }: Props) {
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<number | null>(null);
  const [showDone, setShowDone] = useState(false);
  const [dragId, setDragId] = useState<number | null>(null);
  const report = (e: unknown) => onMessage(String(e));
  const open = openTodos(todos);
  const done = todos.filter((t) => t.state === "done");

  const add = () => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    invoke("todo_add", { root, text }).catch(report);
  };
  const update = (id: number, change: { text?: string; state?: TodoState }) =>
    invoke("todo_update", { root, id, ...change }).catch(report);
  const remove = (ids: number[]) => invoke("todo_remove", { root, ids }).catch(report);
  const copy = async (items: Todo[]) => {
    try {
      const text = await invoke<string>("todo_copy", { root, ids: items.map((t) => t.id) });
      await navigator.clipboard.writeText(text);
      onMessage(
        items.length === 1 ? "Copied: paste it into a prompt" : `Copied ${items.length} to-dos`,
      );
    } catch (e) {
      report(e);
    }
  };
  const dropOn = (onto: number) => {
    if (dragId === null || dragId === onto) return;
    const ids = open.map((t) => t.id).filter((id) => id !== dragId);
    ids.splice(ids.indexOf(onto), 0, dragId);
    invoke("todo_order", { root, ids }).catch(report);
  };

  const row = (t: Todo) => (
    <li
      key={t.id}
      className={`todo ${t.state}${dragId === t.id ? " dragging" : ""}`}
      draggable={t.state !== "done" && editing !== t.id}
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        setDragId(t.id);
      }}
      onDragEnd={() => setDragId(null)}
      onDragOver={(e) => dragId !== null && t.state !== "done" && e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        dropOn(t.id);
      }}
    >
      <input
        type="checkbox"
        checked={t.state === "done"}
        title={t.state === "done" ? "Put back" : "Done"}
        onChange={() => update(t.id, { state: t.state === "done" ? "open" : "done" })}
      />
      {editing === t.id ? (
        <input
          className="todo-edit"
          autoFocus
          defaultValue={t.text}
          onBlur={(e) => (setEditing(null), update(t.id, { text: e.currentTarget.value }))}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setEditing(null);
          }}
        />
      ) : (
        <span
          className="todo-text"
          title="Double-click to edit"
          onDoubleClick={() => setEditing(t.id)}
        >
          {todoParts(t.text).map((p, i) =>
            p.url ? (
              <a
                key={i}
                href={p.url}
                title={p.url}
                onClick={(e) => (e.preventDefault(), void openUrl(p.url!))}
              >
                {p.text}
              </a>
            ) : (
              <span key={i}>{p.text}</span>
            ),
          )}
          {todoNote(t) && <span className="todo-note">{todoNote(t)}</span>}
        </span>
      )}
      {t.state !== "done" && (
        <button className="x" title="Copy as a prompt (marks it sent)" onClick={() => copy([t])}>
          ⧉
        </button>
      )}
      <button className="x" title="Delete" onClick={() => remove([t.id])}>
        ✕
      </button>
    </li>
  );

  return (
    <div className="todos">
      <div className="tasks-head">
        <span>To-do</span>
        <span>
          {open.some((t) => t.state === "open") && (
            <button
              className="x"
              title="Copy every open item as one prompt (marks them sent)"
              onClick={() => copy(open.filter((t) => t.state === "open"))}
            >
              Copy open
            </button>
          )}
          <button className="x" title="Close" onClick={onClose}>
            ✕
          </button>
        </span>
      </div>
      <input
        className="task-filter"
        placeholder="Add a to-do and press Enter"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") add();
          if (e.key === "Escape") onClose();
        }}
      />
      <ul className="todo-list">{open.map(row)}</ul>
      {done.length > 0 && (
        <div className="todo-done">
          <button className="x" onClick={() => setShowDone(!showDone)}>
            {showDone ? "▾" : "▸"} {done.length} done
          </button>
          {showDone && (
            <button
              className="x"
              title="Delete the finished items"
              onClick={() => remove(done.map((t) => t.id))}
            >
              Clear
            </button>
          )}
          {showDone && <ul className="todo-list">{done.map(row)}</ul>}
        </div>
      )}
    </div>
  );
}

/** Who has it: "sent", "doing · Fix the strip", "added by Fix the strip". */
function todoNote(t: Todo): string | null {
  if (t.state === "sent") return "sent";
  if (t.state === "doing") return t.session_name ? `doing · ${t.session_name}` : "doing";
  if (t.state === "done" && t.session_name) return t.session_name;
  if (t.added_by) return `from ${t.added_by}`;
  return null;
}
