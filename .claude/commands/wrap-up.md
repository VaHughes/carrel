---
description: Sync markdown docs with this session's code changes
---
Sync the documentation with what changed this session.

1. Run `git status` and `git diff --stat` (and `git log --oneline -10`
   if changes are already committed) to identify what actually changed.
2. Update docs/STATUS.md: current state, what was completed, what's
   still open, and any decisions made this session. Update the
   "Last updated" line.
3. For docs/ARCHITECTURE.md, docs/DEVELOPMENT.md, and AGENTS.md: only
   edit sections that are now inaccurate because of the changes above.
   Do not reword content that is still correct.
4. Remove documentation that describes code, files, or behavior that
   no longer exists.
5. Finish with a short list of which doc files you touched and why.
