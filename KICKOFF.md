# Kick-off

## Setup (once)

1. Unzip this so the folder is `C:\dev\cc-sidebar` (CLAUDE.md should sit directly inside it).
2. Open Windows Terminal in WSL and run:
   ```bash
   cd /mnt/c/dev/cc-sidebar
   claude --permission-mode auto
   ```
   Auto mode lets the session build, run PowerShell and take screenshots without you approving every step. The safety rules in CLAUDE.md keep it away from your real sessions and global config.
3. Paste the prompt below.

## Opening prompt

```
You're the lead developer on cc-sidebar. Read CLAUDE.md, docs/SPEC.md and docs/PLAN.md first.

Then start M0: run both env checks, tell me in one short list what I need to install or change (with exact commands), and scaffold the Tauri app as far as you can without those. After I confirm the installs, finish M0 and work through the M1 spikes on your own, using screenshots to verify UI behaviour. Check in with me after M1 with findings before starting M2, or earlier if you hit something that needs my decision.
```
