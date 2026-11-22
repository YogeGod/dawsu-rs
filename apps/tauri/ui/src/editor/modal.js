// Shared modal dialog frame and backend-driven dialogs (Save review, Save problems,
// Output review, Pending open/home, Revert, Key preset, Error, Sound-set prompts).
import { describeOutputProblem, nameOf, outputProblemTime } from "./helpers.js";

export function attachModal(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, makeButton, paragraph } = dom;
  const { dialog, grid } = elements;

  function modal(kind, title, content, buttons, cancel = null) {
    if (state.modalKind === kind && dialog.open) return;
    state.modalKind = kind;
    state.modalClose = cancel;
    dialog.className = "dialog-" + kind;
    $("dialog-title").textContent = title;
    $("dialog-body").replaceChildren(...content);
    $("dialog-actions").replaceChildren(...buttons.map(([label, task, primary]) => {
      const button = makeButton(label, task);
      if (primary) button.className = "primary";
      return button;
    }));
    if (!dialog.open) dialog.showModal();
  }

  function closeModal() {
    state.modalKind = "";
    state.modalClose = null;
    dialog.close();
  }

  async function showOutputProblem(index) {
    const problems = state.view.outputProblems ?? [];
    if (!problems.length || state.view.busy) return;
    state.outputProblemIndex = index % problems.length;
    const problem = problems[state.outputProblemIndex];
    if (state.scene && state.scene.path === problem.timeline) {
      await ctx.act({ kind: "seek", ms: problem.time }, false);
      if (state.scene && state.scene.path === problem.timeline) {
        await ctx.changeView(problem.time - grid.clientWidth / state.scene.zoom / 2, state.scene.zoom);
      }
    }
    ctx.closeModal();
    ctx.modal("output-problem", "Problem " + (state.outputProblemIndex + 1) + " of " + problems.length,
      [paragraph(describeOutputProblem(problem))], [
        ["Close", () => ctx.closeModal()],
        ...(problems.length > 1 ? [["Next problem", () => showOutputProblem(state.outputProblemIndex + 1), true]] : []),
      ]);
  }

  function renderDialogs() {
    if (state.view.error) {
      const dismiss = () => { ctx.closeModal(); ctx.act({ kind: "dismiss_error" }); };
      ctx.modal("error", "Editor message", [paragraph(state.view.error)], [["OK", dismiss, true]], dismiss);
    } else if (state.view.saveProblemReview) {
      const problems = state.view.outputProblems ?? [];
      const dismiss = () => { ctx.closeModal(); return ctx.act({ kind: "dismiss_save_problem_review" }); };
      const showProblem = (index) => async () => { await dismiss(); await showOutputProblem(index); };
      // With a reviewed plan, Save writes everything it can and leaves these moments out; without a
      // project nothing could keep them, so Save still refuses.
      const reviewed = Boolean(state.view.saveReview);
      const save = reviewed
        ? () => { ctx.closeModal(); ctx.act({ kind: "answer_save_review", apply: true }); }
        : () => { ctx.closeModal(); ctx.act({ kind: "save" }); };
      const difficulty = env.document.createElement("div");
      difficulty.className = "save-problem-difficulty";
      const label = env.document.createElement("span");
      label.textContent = "Difficulty";
      const file = env.document.createElement("strong");
      file.textContent = nameOf(state.scene?.path ?? "");
      difficulty.append(label, file);
      const list = env.document.createElement("div");
      list.className = "save-problem-list";
      list.setAttribute("role", "list");
      list.append(...problems.map((problem, index) => {
        const row = env.document.createElement("div");
        row.className = "save-problem";
        row.setAttribute("role", "listitem");
        const message = env.document.createElement("span");
        message.textContent = problem.message;
        if (problem.target !== state.scene?.path) {
          const target = env.document.createElement("small");
          target.className = "save-problem-target";
          target.textContent = nameOf(problem.target);
          message.append(target);
        }
        const time = env.document.createElement("time");
        time.textContent = outputProblemTime(problem);
        const button = makeButton("Show", showProblem(index));
        button.setAttribute("aria-label", "Show the problem at " + time.textContent);
        row.append(message, time, button);
        return row;
      }));
      const note = paragraph(reviewed
        ? "These parts won't sound in osu! the way they do here. You can still save; the rest is written as drawn. " +
          "Left-out moments stay in dawsu! and on this list, and Revert to saved brings them back."
        : "These moments can't be written to the difficulty as drawn. Save writes nothing until they are fixed.");
      ctx.modal("save-problems", "Save hitsounds", [difficulty, note, list], reviewed
        ? [["Show first problem", showProblem(0)], ["Cancel", dismiss], ["Save", save, true]]
        : [["Cancel", dismiss], ["Show first problem", showProblem(0)], ["Continue to Save", save, true]], dismiss);
      const first = $("dialog-actions").children[0];
      if (reviewed && first) first.className = "save-problems-show";
      const count = env.document.createElement("span");
      count.className = "problem-count";
      count.textContent = problems.length + (problems.length === 1 ? " problem" : " problems");
      $("dialog-title").replaceChildren("Save hitsounds", count);
    } else if (state.view.saveReview) {
      const answer = (apply) => { ctx.closeModal(); ctx.act({ kind: "answer_save_review", apply }); };
      const files = state.view.saveReview.map(([operation, path]) => paragraph(operation + " · " + path));
      ctx.modal("save-review", "Review Save", [
        paragraph("Editing " + nameOf(state.scene.path)),
        ...files.length ? files : [paragraph("No files need to change.")],
      ], [["Cancel", () => answer(false)], [files.length ? "Save" : "Done", () => answer(true), true]], () => answer(false));
    } else if (state.view.pendingHome) {
      const resolve = (choice) => { ctx.closeModal(); ctx.act({ kind: "home_choice", choice }); };
      ctx.modal("pending-home", "Save changes before going Home?", [paragraph("The open difficulty has unsaved changes.")],
        [["Cancel", () => resolve("cancel")], ["Discard", () => resolve("discard")], ["Save", () => resolve("save"), true]],
        () => resolve("cancel"));
    } else if (state.view.pendingOpen) {
      const resolve = (choice) => { ctx.closeModal(); ctx.act({ kind: "pending", choice }); };
      ctx.modal("pending", "Save changes before opening?", [paragraph("The open difficulty has unsaved changes.")],
        [["Cancel", () => resolve("cancel")], ["Discard", () => resolve("discard")], ["Save", () => resolve("save"), true]],
        () => resolve("cancel"));
    } else if (state.view.revert) {
      const cancel = () => { ctx.closeModal(); ctx.act({ kind: "cancel_revert" }); };
      ctx.modal("revert", "Revert to saved difficulty?", [paragraph("This reloads the saved difficulty from disk.")],
        [["Cancel", cancel], ["Revert", () => { ctx.closeModal(); ctx.act({ kind: "confirm_revert" }); }, true]], cancel);
    } else if (state.view.review) {
      const answer = (apply) => { ctx.closeModal(); ctx.act({ kind: "output", apply }); };
      const heading = env.document.createElement("h3");
      heading.textContent = "Files this action will write";
      const files = state.view.review.files.flatMap((file) => [
        paragraph((file.replace ? "Replace: " : "Create: ") + file.path),
        paragraph(file.changed + " objects changed · " + file.added + " added · " + file.removed +
          " removed · " + file.timing + " timing lines affected"),
      ]);
      const details = state.view.review.details.map(paragraph);
      const mixes = (state.view.review.mixes ?? []).map((path, index) => {
        const row = env.document.createElement("div");
        row.className = "row";
        const label = paragraph(path);
        row.append(label, makeButton("Audition mix", () => ctx.act({ kind: "audition_output_mix", index }, false)));
        return row;
      });
      const mixHeading = env.document.createElement("h3");
      mixHeading.textContent = "Sample mixes";
      const mixSection = mixes.length ? [mixHeading, ...mixes] : [];
      const combinations = state.view.review.combinations ?? [];
      const includeSliderAudio = Boolean(state.view.review.includeSliderAudio);
      const choices = combinations.length ? [["Edit merge rules…", () => {
        ctx.closeModal();
        ctx.run(async () => {
          await ctx.update({ kind: "output", apply: false });
          await ctx.openMergeSettings(combinations, true, includeSliderAudio);
        });
      }]] : [];
      const source = state.scene ? [paragraph("Editing " + nameOf(state.scene.path))] : [];
      ctx.modal("review", state.view.review.label, [...source, heading, ...files, ...mixSection, ...details],
        [["Cancel", () => answer(false)], ...choices, ["Write files", () => answer(true), true]], () => answer(false));
    } else if (state.view.editDialog && state.scene) {
      ctx.showTimelineEdit(state.view.editDialog);
    } else if (state.scene && state.view.soundSetPrompts?.length && env.pendingSoundSetPrompt?.()) {
      env.showSoundSetPrompt?.(env.pendingSoundSetPrompt());
    } else if (state.view.firstLaunch) {
      const choose = (name) => ctx.act({ kind: "key_preset", name });
      ctx.modal("key-preset", "Choose your keyboard preset", [
        paragraph("osu! lets you seek through the beat grid with the arrow keys."),
        paragraph("DAW-like uses the editor’s default timeline shortcuts."),
      ], [["osu!", () => choose("osu!"), true], ["DAW-like", () => choose("DAW")]], () => {});
    } else if (["error", "pending", "pending-home", "revert", "review", "save-review", "save-problems", "key-preset", "timeline-edit", "sound-set-update"].includes(state.modalKind)) {
      ctx.closeModal();
    }
  }

  function bindModal() {
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      if (state.modalClose) state.modalClose();
      else ctx.closeModal();
    });
  }

  Object.assign(ctx, { modal, closeModal, showOutputProblem, renderDialogs, bindModal });
}
