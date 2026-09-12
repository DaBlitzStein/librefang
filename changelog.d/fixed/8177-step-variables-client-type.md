The dashboard API client now describes the per-step `variables` map that the run-detail route emits.
`StepResult::variables` is serialised by `routes/workflows/workflow.rs` and covered end to end by a lifecycle test, but `WorkflowStepResult` in `api.ts` did not declare it, so the bindings were discarded at the TypeScript boundary and any surface wanting to show them had to cast.
This is the client half only; rendering them lives with the run timeline in #7997 (#8177) (@DaBlitzStein)
