#!/usr/bin/env python3
"""PreToolUse hook for Bash tool calls.

Blocks `git commit` if the staged diff contains PII (real names, unexpected
email addresses) per the deny-list in pii-denylist.txt. Self-filters on the
command string from stdin: only scans when the command contains
"git commit" - a no-op for every other Bash call.

Exit codes:
  0 = allow (not a commit, or nothing blocking found)
  2 = block (a "block" term or an unexpected email was found)
"""
import json
import os
import re
import subprocess
import sys

ALLOWED_EMAIL = "kkodecs@proton.me"

# This file's own job is to contain the deny-listed terms as config entries,
# not to leak them - exclude it from its own scan.
EXEMPT_FILES = {"pii-denylist.txt"}


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        sys.exit(0)

    command = data.get("tool_input", {}).get("command", "")
    if "git commit" not in command:
        sys.exit(0)

    hook_dir = os.path.dirname(os.path.abspath(__file__))
    denylist_path = os.path.join(hook_dir, "pii-denylist.txt")

    env = dict(os.environ)
    env["RTK_DISABLED"] = "1"
    try:
        result = subprocess.run(
            ["git", "diff", "--cached", "-U0"],
            capture_output=True, text=True, env=env, check=False,
        )
        diff_output = result.stdout
    except Exception:
        sys.exit(0)

    added_lines = []
    current_file_exempt = False
    for line in diff_output.splitlines():
        if line.startswith("diff --git "):
            # e.g. "diff --git a/.claude/hooks/pii-denylist.txt b/.claude/hooks/pii-denylist.txt"
            parts = line.split()
            path = parts[-1][2:] if parts else ""
            current_file_exempt = os.path.basename(path) in EXEMPT_FILES
            continue
        if current_file_exempt:
            continue
        if line.startswith("+++"):
            continue
        if line.startswith("+"):
            added_lines.append(line[1:])

    if not added_lines:
        sys.exit(0)

    added_text = "\n".join(added_lines)
    findings = []

    if os.path.isfile(denylist_path):
        with open(denylist_path) as f:
            for raw in f:
                raw = raw.strip()
                if not raw or raw.startswith("#"):
                    continue
                mode, sep, term = raw.partition(":")
                if not sep or not term:
                    continue
                pattern = re.compile(r"\b" + re.escape(term) + r"\b", re.IGNORECASE)
                hits = [l for l in added_lines if pattern.search(l)]
                if hits:
                    findings.append((mode.strip(), f"term '{term}': {hits[:3]}"))

    email_pattern = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
    emails = {m.group(0) for m in email_pattern.finditer(added_text)}
    bad_emails = {e for e in emails if e.lower() != ALLOWED_EMAIL}
    if bad_emails:
        findings.append(("block", f"unexpected email(s): {sorted(bad_emails)}"))

    if not findings:
        sys.exit(0)

    blocking = any(mode == "block" for mode, _ in findings)
    lines = [f"[{mode}] {msg}" for mode, msg in findings]
    reason = "PII check on staged diff:\n" + "\n".join(lines)

    if blocking:
        print(reason, file=sys.stderr)
        print(
            "Fix the flagged content, or edit .claude/hooks/pii-denylist.txt "
            "if this is a false positive.",
            file=sys.stderr,
        )
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            },
            "systemMessage": reason,
        }))
        sys.exit(2)
    else:
        print(reason, file=sys.stderr)
        print(json.dumps({"systemMessage": reason}))
        sys.exit(0)


if __name__ == "__main__":
    main()
