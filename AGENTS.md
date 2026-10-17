# Orion working agreements

## Teaching-first development

Orion is a learning project. Complete the authorized task. Explain decisions that affect operation or maintenance.

Before a related set of changes, explain the affected files and intended behavior. Keep each change small enough to review.

Explain resource paths, joint coordinates, controller interfaces, package boundaries and simulator assumptions before changing them. State the invariant that the change preserves. Describe the check that protects it.

Continue through authorized work after each explanation. Pause when the user requests a guided exercise or review before implementation.

## Project context

Use [the documentation index](docs/README.md) to find relevant project documents. Confirm their behavior against code, configuration and component instructions.

## Documentation changes

### Writing rules

- Write in ASD-STE100 style. Use one topic per paragraph. Use at most six sentences per paragraph.
- Use at most 20 words in procedural sentences. Use at most 25 words in descriptive sentences.
- Give one instruction per sentence. Use imperative verbs for procedures. Use active voice and present tense.
- Use each word with one meaning. Prefer common words. Treat joint names, commands and file names as technical nouns.
- Use numbered steps for procedures. Put each command in a fenced block. Put one sentence before each command to explain its function.
- Describe what Orion does and how to use, build or change it.
- Delete verification status, measurement dates, capture dates, commissioning records, test evidence, review history and experiment results. Delete explanations of how values were found. Do not move this material to another document.
- Replace jargon with common words. Define each necessary technical term in one short sentence at its first use.
- Put a WARNING note immediately before a step that has an injury risk. Put a CAUTION note immediately before a step that has a damage risk.

### File responsibilities

- `README.md` gives the overview, system summary, repository map, first actions, common checks and safety guidance.
- `docs/README.md` contains a compact link index.
- Architecture documents describe data flow, boundaries, state changes, invariants and design choices.
- Reference documents contain configuration, schemas, control details and the animation catalogue.
- `docs/learning_notes/` teaches Orion concepts. Link to code or configuration for exact values.
- Component READMEs describe only local behavior, setup, commands and constraints. Put cross-system explanations in `docs/`.

### Maintenance rules

- Keep each changing fact in one maintained location. Link to that location elsewhere. Repeat safety notes when necessary.
- Keep only information that helps a reader understand, operate, troubleshoot or develop Orion.
- Check commands, paths, flags, file names and links against the code. Remove statements that contradict it.
- State platform and hardware restrictions beside the affected procedure.
- Update instructions when behavior, paths, commands or dependencies change.
- Remove obsolete pages after preserving useful operating information.
- Use meaningful relative links and sentence-case headings. Give each page one level-one heading. Give fenced blocks a language identifier.
