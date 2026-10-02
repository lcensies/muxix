---
description: Signal that this pane's gate is rejected, with optional feedback (the stage re-runs)
argument-hint: [feedback]
allowed-tools: Bash(workmux signal reject:*)
---

!`workmux signal reject --feedback "$ARGUMENTS"`

Wrote the `reject` signal for this pane, with the feedback above (if any).
Whatever harness is waiting on this gate decides what to re-run.
