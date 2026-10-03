---
description: Approve the current workmux pipeline gate and proceed (e.g. plan → implement)
allowed-tools: Bash(workmux signal proceed:*)
---

!`workmux signal proceed`

Signalled the workmux pipeline to proceed past the current gate. If a gate node is
waiting (e.g. after planning), it will now advance to the next stage.
