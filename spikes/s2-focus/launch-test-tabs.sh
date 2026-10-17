#!/usr/bin/env bash
# Open a NEW Windows Terminal window (named cc-sidebar-test) with two test tabs running claude
# in sandbox repos. The command per tab avoids ';' because wt.exe splits on it.
set -e
t="$(dirname "$(readlink -f "$0")")/tab.sh"
tab() { echo "new-tab --title cc-test wsl.exe -d $WSL_DISTRO_NAME -- bash -l $t $1 $2"; }
cmd.exe /c "wt.exe -w cc-sidebar-test $(tab test1 alpha) ; $(tab test2 bravo)"
