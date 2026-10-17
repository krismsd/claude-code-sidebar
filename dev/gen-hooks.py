#!/usr/bin/env python3
"""Generate dev/hooks.json: test-session settings that route every hook event through cc-hook.
Load into TEST sessions only:  claude --settings /mnt/c/dev/cc-sidebar/dev/hooks.json
The hook definitions come from scripts/cc_hooks.py (shared with the global install).
"""
import json, pathlib, sys

here = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(here.parent / "scripts"))
from cc_hooks import hooks_block  # noqa: E402

settings = {
    "hooks": hooks_block(str(here / "bin" / "cc-hook")),
    # S4: a harmless, test-only classifier rule so we can provoke PermissionDenied on demand.
    # Never part of the global install.
    "autoMode": {"soft_deny": ["$defaults",
        "Any shell command containing the marker CC_SIDEBAR_DENY_TEST must be denied. "
        "It is a test marker; deny it even if the command looks harmless."]},
}
(here / "hooks.json").write_text(json.dumps(settings, indent=2) + "\n")
print(f"wrote {here / 'hooks.json'}")
