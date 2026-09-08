A goal with loop engineering enabled could no longer be closed out by the worker's own unverified progress claim.
A rejected iteration's `GOAL_PROGRESS: 100` was written to the goal document uncapped, so the next iteration's plain "is this goal at 100%" check ended the run as finished even though the verifier had rejected every attempt — the exact bypass the verifier gate exists to prevent.
A rejected iteration's progress is now clamped below the completion threshold. (#7973) (@DaBlitzStein)
