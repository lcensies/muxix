---
description: Reject the current workmux pipeline gate with optional feedback (re-runs the stage)
argument-hint: [feedback]
allowed-tools: Bash(workmux signal reject:*)
---

!`workmux signal reject --feedback "$ARGUMENTS"`

Signalled the workmux pipeline to reject the current gate. The stage will re-run with
the feedback above (if any).
