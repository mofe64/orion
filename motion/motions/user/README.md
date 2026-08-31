# Orion user motions

Orion Studio stores user-authored named motions here. Each motion references
named poses; it never contains raw servo registers or a joint command stream.
Commissioned functional and expressive motions remain in their existing
directories and cannot be replaced.

User motions are immutable after creation. Orion continues to generate each
transition with its runtime quintic trajectory and measured completion logic.
