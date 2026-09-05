# Orion Working Agreements

## Teaching-first development

Orion is a learning project. Complete the authorized work while explaining
enough of the mechanism for me to understand and reproduce it.

Before each coherent change, explain the affected files' roles, the behavior
being changed, and why the change is needed. Keep changes small enough to
review, but group files that implement one architectural idea.

Explain resource paths, coordinate conventions, joint names and limits,
controller interfaces, package boundaries, and simulator assumptions before
changing them. Include the relevant invariant and how it will be verified.

Continue through authorized steps after explaining them. Pause at checkpoints
when I explicitly request a guided exercise or review before implementation.

## Project context

Before substantial work, read `docs/project/status.md` and
`docs/project/roadmap.md`, then confirm the relevant behaviour against code,
tests, and component instructions.

## Documentation changes

### Reader-first writing

- Write only what helps someone understand, operate, troubleshoot, or develop
  Orion.
- Lead with the Orion fact, outcome, prerequisite, or action. Do not narrate
  why a page exists or how the documentation was organised.
- Avoid openings such as “This page explains,” “This tutorial covers,” “source
  of truth,” or “the following documentation.”
- Use direct, active sentences and present tense for implemented behaviour.
- Distinguish implemented, partial, and planned behaviour explicitly. Do not
  use vague status words such as “new,” “currently,” or “next.”
- Define an unfamiliar term when it first appears, but do not explain basic
  syntax the reader already needs for the procedure.

### File responsibilities

- `README.md` gives the product overview, system summary, repository map,
  first actions, common validation, and safety warning.
- `docs/README.md` is a compact link index. It must not explain the
  documentation taxonomy or contributor process.
- `docs/tutorials/` provides complete first-run paths with prerequisites,
  working directories, commands, expected results, and safe stopping points.
- `docs/how-to/` provides focused procedures for one operational outcome.
- `docs/explanation/` describes Orion's architecture, data flow, boundaries,
  state transitions, invariants, and design rationale.
- `docs/reference/` contains exact compatibility, configuration, protocol,
  licensing, and platform facts.
- `docs/project/status.md` lists implemented, partial, and planned capability.
  `docs/project/roadmap.md` orders remaining product outcomes.
- `docs/decisions/` records cross-component decisions using context, decision,
  consequences, and status.
- `docs/learning_notes/` teaches Orion concepts while linking to the code or
  configuration that supplies exact values.
- Component READMEs contain only behaviour, setup, commands, and constraints
  local to that directory. Cross-system explanations belong under `docs/`.

### Accuracy and maintenance

- Give each changing fact one maintained location and link to it elsewhere.
  Repeat only safety warnings that a reader could otherwise miss.
- Keep product status out of component feature descriptions unless the status
  directly changes how the component is used.
- State platform and hardware restrictions beside the affected command.
- Treat code, manifests, configuration, tests, and commissioned hardware
  evidence as authoritative; do not preserve prose that contradicts them.
- Update affected instructions in the same change as behaviour, configuration,
  file paths, commands, or dependencies.
- Remove obsolete pages after preserving any still-valid product information.
  Keep an archive notice only when active code or an operational procedure
  still depends on the archived material.
- Use meaningful relative links, sentence-case headings, one level-one heading
  per page, and language identifiers on fenced code blocks.
