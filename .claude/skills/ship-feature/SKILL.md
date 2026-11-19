---
name: ship-feature
description: Plan, build, and check one dawsu! Rust feature in separate sessions, with the approved plan as the handoff.
argument-hint: "plan <feature or tracker item> | build | check"
disable-model-invocation: true
---

# Ship a dawsu! feature

Run one phase per session. Use `/ship-feature plan <feature or tracker item>`, `/ship-feature build`, or `/ship-feature check`. With no phase specified, read the status in `.handoff/PLAN.md`. An absent plan starts planning. An approved plan starts building. A built plan starts checking. A checked plan returns its results.

## Plan and handoff

1. Read the tracker item, the checkout's instructions, and the accepted design for the area. The rewrite tracker is `dawsu-rewrite/todo.md` in the Obsidian vault. Use `DESIGN.md` and `FEATURE-COVERAGE.md` in the Rust checkout where present. Check Git status and identify the intended base and checkout.
2. Resolve the decisions required for the work. Use grilling when available and a small reproduction or prototype when a decision needs runnable evidence.
3. Show the goal, files, observable finish, and checks. Wait for approval of that plan. An existing approval that covers the same plan is enough.
4. Use the approved checkout. If the plan calls for a new worktree, create it from the recorded base after approval. Copy any needed corpus inputs as described in [verify-dawsu](../verify-dawsu/SKILL.md).
5. Keep `.handoff/` ignored in the chosen checkout. Write `.handoff/PLAN.md` with status **approved, ready for build**. Record branch, checkout path, base commit, starting Git status, approved goal and files, finish criteria, verification commands, and decisions with source links. Include sections for the build log, check results, and remaining manual checks. Point `.handoff/HANDOFF.md` at the plan and finish the planning session.

## Build

1. Read `.handoff/PLAN.md` and the files it names. Start only when its status is **approved, ready for build**. Confirm the branch and base and preserve other work shown in the starting status. If a new design choice is needed, record it and return to planning.
2. Build the approved change and keep a short log in the plan. Use existing targeted tests and test-driven development at agreed public interfaces where applicable.
3. Run the plan's quick checks. Record the results and set the status to **built, ready for check**. Finish the build session with the plan path.

## Check

1. Read the plan in a fresh session. Start only when its status is **built, ready for check**. Compare the planned work with the recorded base, including staged, unstaged, and new files in scope.
2. Run [verify-dawsu](../verify-dawsu/SKILL.md). For map load, save, compile, decompile, or Copy changes, apply [format-guardian](../../agents/format-guardian.md). Apply [scope-reviewer](../../agents/scope-reviewer.md) to the goal and diff. Review inline unless the user asks for subagents.
3. Review correctness, standards, and scope with the available review skill or the repository's checklist. Fix findings within the plan and rerun affected checks. Return to planning if a fix needs a new design decision.
4. Record the final commands, counts, and any outstanding manual checks. Once the plan's finish criteria are met, update the tracker and affected feature coverage and set the status to **checked, ready for the user**.
5. Present the result and wait for the user's direction on Git delivery. Commit only on an explicit request, and put the vault revision that updated the design in the commit message when applicable. Treat a push, publication, or opening a pull request as a separate explicit action.
