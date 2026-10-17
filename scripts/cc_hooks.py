#!/usr/bin/env python3
"""Merge cc-sidebar's hooks into a Claude Code settings.json, or take them out again.

Our entries are recognised by their command running `cc-hook`, so everything else in the file
(other hooks, settings) is left untouched. Writes are atomic and the original is backed up.

  cc_hooks.py install   [--settings F] [--shim PATH] [--dry-run]
  cc_hooks.py uninstall [--settings F] [--dry-run]
  cc_hooks.py print     [--shim PATH]        # just our hooks block, as JSON
"""
import argparse, copy, difflib, json, os, pathlib, sys, tempfile, time

EVENTS = ["SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse",
          "PostToolUseFailure", "PermissionRequest", "PermissionDenied", "Notification", "Stop",
          "StopFailure", "PreCompact", "CwdChanged", "SubagentStart", "SubagentStop"]
# Hook timeouts default to 600 s, so always set one; cc-hook's curl limit sits below it.
# PermissionRequest is held while the terminal prompt is open (SPEC §3.6): the app releases it
# at 575 s, the shim gives up at 580 s, and Claude Code kills the hook at 590 s.
LIMITS = {"PermissionRequest": (580, 590)}  # (curl max seconds, hook timeout seconds)
DEFAULT = (3, 5)


def ours(entry: dict) -> bool:
    return any("cc-hook" in h.get("command", "") for h in entry.get("hooks", []))


def hooks_block(shim: str) -> dict:
    block = {}
    for ev in EVENTS:
        curl_max, timeout = LIMITS.get(ev, DEFAULT)
        block[ev] = [{"hooks": [{"type": "command", "command": f"{shim} {curl_max}", "timeout": timeout}]}]
    return block


def uninstall(settings: dict) -> dict:
    s = copy.deepcopy(settings)
    hooks = s.get("hooks", {})
    for ev in list(hooks):
        kept = [e for e in hooks[ev] if not ours(e)]
        if kept:
            hooks[ev] = kept
        else:
            del hooks[ev]
    if "hooks" in s and not s["hooks"]:
        del s["hooks"]
    return s


def install(settings: dict, shim: str) -> dict:
    s = uninstall(settings)  # idempotent: replace any previous version of ours
    hooks = s.setdefault("hooks", {})
    for ev, entries in hooks_block(shim).items():
        hooks.setdefault(ev, []).extend(entries)
    return s


def write(path: pathlib.Path, data: dict) -> pathlib.Path | None:
    backup = None
    if path.exists():
        backup = path.with_name(f"{path.name}.cc-sidebar-backup-{time.strftime('%Y%m%d-%H%M%S')}")
        backup.write_bytes(path.read_bytes())
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".settings.", suffix=".tmp")
    with os.fdopen(fd, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    json.loads(pathlib.Path(tmp).read_text())  # sanity check before replacing
    os.replace(tmp, path)
    return backup


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["install", "uninstall", "print"])
    ap.add_argument("--settings", default=str(pathlib.Path.home() / ".claude" / "settings.json"))
    ap.add_argument("--shim", default=str(pathlib.Path.home() / ".local" / "bin" / "cc-hook"))
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    if a.action == "print":
        print(json.dumps({"hooks": hooks_block(a.shim)}, indent=2))
        return 0
    path = pathlib.Path(a.settings)
    before = json.loads(path.read_text()) if path.exists() else {}
    after = install(before, a.shim) if a.action == "install" else uninstall(before)
    if after == before:
        print(f"{path}: no changes")
        return 0
    diff = difflib.unified_diff(json.dumps(before, indent=2).splitlines(), json.dumps(after, indent=2).splitlines(),
                                f"{path} (before)", f"{path} (after)", lineterm="", n=1)
    if a.dry_run:
        print("\n".join(diff))
        return 0
    backup = write(path, after)
    print(f"{a.action}ed cc-sidebar hooks in {path}" + (f" (backup: {backup})" if backup else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
