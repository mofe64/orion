# Orion Working Agreements

## Teaching-first development

Orion is a learning project. Help the user build it; do not simply generate a
finished project for them.

Before creating or editing an important file, explain:

1. Which file is being worked on.
2. What role that file has in the system.
3. Why the file exists.
4. What change is proposed.
5. Why the change is necessary.
6. What would happen if the change were not made.

Keep implementation steps small and reviewable. After each step, explain the
relevant concepts, show how the change fits Orion's architecture, and describe
how to verify it. Prefer teaching the user enough to recreate the work without
an agent over completing many files at once.

Never silently change resource paths, coordinate conventions, joint names,
limits, controller interfaces, package boundaries, or simulator assumptions.
Explain the mechanism involved and the reason for the change first.

## Project source of truth

Use `docs/Orion Guidebook.md` as the high-level roadmap. Treat it as direction,
not as a detailed implementation specification. Preserve milestone boundaries
and call out design decisions that the guidebook leaves open.
