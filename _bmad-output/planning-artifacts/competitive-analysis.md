# Competitive Analysis — LibreFang vs Agent Frameworks

**Date:** 2026-05-24  |  **Author:** Blitz  |  **Status:** Complete

## LibreFang's Unique Position

Only self-hosted open-source Agent OS with governance-grade features (RBAC, audit hash chain, 4-layer tool gate, approval workflows, capability ceiling) + 20+ native channels + P2P federation (OFP). Rust runtime = near-zero idle cost.

## Framework Comparison Matrix

| Capability | CrewAI | LangGraph | DeepAgents | AG2 | OpenAI SDK | Bedrock | Google | Hermes | **LibreFang** |
|---|---|---|---|---|---|---|---|---|---|
| Self-hosted | Yes | Yes | Yes | Yes | Yes | No | No* | Yes | **Yes** |
| RBAC | Ent only | Partial | No | No | No | Yes | Yes | No | **Yes** |
| Audit trail | Partial | Yes | Yes | Partial | Yes | Yes | Yes | Partial | **Yes (hash chain)** |
| Budget/token limits | No | No | No | No | No | Partial | Partial | Partial | **Yes** |
| Channels (20+) | No | No | No | No | No | No | No | Yes | **Yes** |
| P2P Federation | No | No | No | No | No | No | Partial | No | **Yes (OFP)** |
| Deterministic workflows | Yes | Yes | Partial | Partial | No | Partial | Yes | No | **Yes** |
| Multi-provider | Yes | Yes | Yes | Yes | Partial | Partial | Partial | Yes | **Yes** |
| Crash recovery | Partial | Yes | Yes | No | Partial | Yes | Yes | Yes | **Yes** |
| Rust (zero idle) | No | No | No | No | No | N/A | N/A | No | **Yes** |

## Gaps to Close

1. A2A protocol support (Google/Linux Foundation, 150+ orgs)
2. Visual workflow designer maturity (LangGraph/Bedrock level)
3. Pluggable memory backends (Hermes level)
4. Pre-session capability restriction (Aethelgard paper)
5. Adaptive autonomy (Anthropic empirical data)

## Compliance

- Certifications are FRAMEWORK-agnostic — apply to controls, not deployment
- LibreFang edge: vendor independence + LLM data control + on-prem for mandated jurisdictions
- Bedrock: FedRAMP High/HIPAA/PCI — covers PLATFORM, not agent BEHAVIOR
