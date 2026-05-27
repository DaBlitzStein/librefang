# LibreFang Product Vision

**Date:** 2026-05-24
**Author:** Blitz
**Status:** Definitive

## Core Identity

**LibreFang is an Agent Operating System, not a framework.**

Like Linux doesn't tell you how to use processes, LibreFang doesn't tell you how to use agents. It provides the kernel primitives, and users/operators configure autonomy levels per use case.

## The Autonomy Spectrum

```
DETERMINISTIC <<<<<<<<<<<<<<<<<<>>>>>>>>>>>>>>>>>>>> AUTONOMOUS
(workflow)              LibreFang              (hermes/openclaw)
```

| Autonomy Level | Use Case | Example |
|---|---|---|
| Low | Corporate SDLC | Steps, approvals, audit trail |
| Medium | Document drafting | Creative within templates |
| Medium-High | Expert remediation | Diagnose freely, escalate for action |
| High | Personal email/calendar | Read, classify, respond |
| Maximum | POC/exploration | Do whatever it takes |
| Mixed | End-to-end SDLC | Different autonomy per phase |

## Kernel Responsibilities

1. **Resource management** — tokens, memory, providers, system capabilities
2. **Isolation & protection** — permissions, budgets, capability ceiling, audit
3. **IPC** — agents talk to each other and to humans (channels, agent_send)
4. **Scheduling** — triggers, cron, queues, priorities
5. **NO opinion on usage** — that's the operator's job

## Key Design Decisions

- Agents are REUSED across sessions, not spawned per task
- Scaling = task queues (Kafka-style exactly-once), not more agent instances
- System capabilities = installable packages, same pattern as MCP/skills
- "Hands" = packaging/marketplace layer for agent configs, not a runtime concept
- Autonomy level = governance configuration, not agent type
- Workflow designer = APPLICATION on the OS, not the OS itself
