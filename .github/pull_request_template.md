<!-- docs/developer/working.md. A typo or one-line wording change needs only the last three. -->

**Impact:** the rules and the code that owns them this touches, and the other features that use them.

**Failing first:** the tests that failed before the change and pass after.

**Saved output checked independently:** the saved-output checker or Poppler, for anything that removes or keeps
content; or "not applicable".

**Checks run:** what passed, what failed and what wasn't run.

**Docs updated:**

**Staged diff reviewed:** `git diff --cached --stat`
