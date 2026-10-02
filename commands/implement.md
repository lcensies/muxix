---
description: Signal that this pane's gate is approved and work may proceed (e.g. plan → implement)
allowed-tools: Bash(workmux signal proceed:*)
---

!`workmux signal proceed`

Wrote the `proceed` signal for this pane. Whatever harness is waiting on this
gate (after a plan, before a merge, ...) sees it and advances.
