# PRD Requirements Brain Dump

**Date:** 2026-05-24 (updated 2026-05-27)  |  **Author:** Blitz  |  **Status:** Raw — needs PRD formalization

## Taxonomía Fundamental

```
┌─────────────────────────────────────────────────────────────┐
│                        KERNEL LAYER                         │
├─────────────────────────────────────────────────────────────┤
│  Agent        Primitiva atómica con LLM                     │
│  Goals        Objetivos scoped (system/team/agent)          │
│  IPC          agent_send, channels, events                  │
│  Scheduling   triggers, cron, queues                        │
│  Resources    tokens, memory, providers, capabilities       │
└─────────────────────────────────────────────────────────────┘
```

---

## 1. Agent Architecture

**Capa:** Kernel

- Primitiva atómica: un agent = una identidad + config + session(s)
- Immutable: id, created_at
- Mutable: skills, tools, channels, MCPs, models, providers, autonomy level
- Auto-evolve: configurable per-agent (PRs #5678, #5741)
- Agents se REUSAN entre sesiones, no se spawnan por tarea
- Scaling = colas de tareas, no más instancias

## 2. Provider & Token Management

**Capa:** Kernel (Resources)

- Per-PROVIDER per-AGENT limits (tokens, no dinero)
- Agent tiene N providers válidos (no primary+fallback)
- Global defaults heredados salvo override
- Gateways (Kong, LiteLLM): desactivar límites internos
- Retry con backoff incremental

## 3. Memory Backends

**Capa:** Kernel (Resources)

- Backend actual: SQLite (substrate)
- Backends pluggables: Honcho, Mem0, etc.
- Per-agent memory isolation
- Session state vs long-term memory separados

## 4. Routing

**Capa:** Kernel (IPC)

- RFC #5671, Issue #5323
- HITL vs AITL topology
- Per-conversation agent assignment
- Per-channel/instance configurable
- Thread ownership registry

## 5. Teams & Workflows

**La ÚNICA diferencia es QUIÉN ORQUESTA:**

| | **Team** | **Workflow** |
|---|----------|--------------|
| Orquesta | **Agent** (LLM, autónomo) | **Código/DAG** (determinista) |
| Agentes | ✓ | ✓ |
| Software/tools | ✓ | ✓ |
| Goals | ✓ | ✓ |
| Inyectable | ✓ | ✓ |
| Templates | ✓ | ✓ |

### Team

Grupo de agentes orquestados por un **agent orquestador**:
- El orquestador decide dinámicamente qué agent hace qué
- Decisiones basadas en contexto, LLM reasoning
- Puede adaptarse a situaciones inesperadas
- Composición: N agents + 1 orchestrator agent

```toml
[team]
name = "customer-support"
version = "1.2.0"
orchestrator = "triage-agent"

[[agents]]
role = "triage"
template = "classifier-agent"

[[agents]]
role = "specialist"
template = "domain-expert"
count = 3
```

### Workflow

Grupo de agentes orquestados por **código/DAG**:
- Pasos predefinidos, condicionales explícitos
- Determinista y auditable
- No se adapta fuera de los branches definidos
- Composición: N agents + DAG definition

```yaml
workflow:
  name: document-review
  steps:
    - id: extract
      agent: extractor
      on_success: validate
    - id: validate
      agent: validator
      on_success: approve
      on_failure: escalate
    - id: approve
      agent: approver
    - id: escalate
      agent: human-reviewer
```

### Ambos pueden:
- Inyectarse dentro de otro Team o Workflow
- Tener Goals asociados (scope Team o scope de los agents individuales)
- Llamar a software/packages externos
- Ser instanciados desde templates del marketplace

### Cuándo usar cada uno

| Caso de uso | Recomendación |
|-------------|---------------|
| Proceso regulado con audit trail | Workflow |
| Triage dinámico basado en contenido | Team |
| CI/CD pipeline | Workflow |
| Investigación abierta | Team |
| Onboarding con pasos fijos | Workflow |
| Soporte al cliente | Team |

## 6. Skill Evolution (Hermes Parity)

Ver `hermes-skill-evolution-plan.md`:
1. Enhanced review prompt
2. Usage telemetry
3. Lifecycle states
4. Curator agent
5. Channel notifications
6. Dashboard UI

## 7. Event Bus

**Capa:** Kernel (IPC)

- Pub/sub interno para comunicación desacoplada
- Eventos tipados: `agent.*`, `goal.*`, `team.*`
- Subscripciones por agent, team, o global
- Hook points para integraciones externas
- Apps de userspace emiten sus propios eventos (e.g., `workflow.*`)

### Garantías de Entrega

**Decisión:** At-least-once por defecto.

- Subscribers DEBEN ser idempotentes
- Eventos incluyen `event_id` para deduplicación client-side

```rust
pub struct Event {
    pub id: EventId,
    pub event_type: String,
    pub payload: Value,
    pub timestamp: DateTime,
    pub source: EventSource,
}
```

## 8. State & Checkpoints

**Capa:** Kernel (Resources)

- Generic KV state storage
- Apps lo usan para checkpoints, snapshots, lo que quieran
- Session state (efímero) vs memory (persistente)
- Rollback/restore capabilities

## 9. Goals Jerárquicos

**Capa:** Kernel

### Scopes

```
System → Team → Agent
```

(Workflow goals = goals del orchestrator agent o de los agents individuales)

### Modelo

```rust
pub enum GoalScope {
    System,
    Team(TeamId),
    Agent(AgentId),
}

pub struct Goal {
    id, title, description,
    parent_id: Option<GoalId>,
    scope: GoalScope,
    owner_id: Option<AgentId>,
    assignee_ids: Vec<AgentId>,
    status, progress, priority, due_at,
    tags: Vec<String>,
    created_at, updated_at,
}
```

### Visibilidad

| Scope | Ve | Edita |
|-------|-----|-------|
| System | Todos | Admin/owner |
| Team | Miembros del team | Miembros |
| Agent | Solo él | Solo él |

### Propagación de Progreso

**Decisión:** NO automática. El orchestrator (agent o workflow) lo hace explícitamente si quiere.

## 10. Cluster & Scaling

**Capa:** Kernel

- Raft-core para consenso
- Task queues (Kafka-style exactly-once)
- Peer discovery
- State replication

## 11. Permissions

**Capa:** Kernel

Falta definir:
- ¿Quién puede crear System goals?
- ¿Quién puede instanciar Teams/Workflows?
- ¿Quién es "miembro" de un Team?
- Roles: admin, operator, agent, external?

---

## Decisiones Tomadas

| Pregunta | Decisión | Rationale |
|----------|----------|-----------|
| Team vs Workflow | Difieren SOLO en orquestación | Agent (autónomo) vs Código (determinista) |
| Goal scopes | System, Team, Agent | Workflow goals = agent goals |
| Event bus delivery | At-least-once | Simple, subscribers idempotentes |
| Progress propagation | No automática | Orquestador lo hace explícito |
