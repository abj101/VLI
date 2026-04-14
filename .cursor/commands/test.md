---
description: Run TDD workflow — write failing tests, implement, verify. For bugs, use the Prove-It pattern.
---

**Cursor:** Read `%USERPROFILE%\.cursor\agent-skills\skills\test-driven-development\SKILL.md` (clone [agent-skills](https://github.com/addyosmani/agent-skills) there if missing) and follow that workflow. Original stub:
Invoke the agent-skills:test-driven-development skill.

For new features:
1. Write tests that describe the expected behavior (they should FAIL)
2. Implement the code to make them pass
3. Refactor while keeping tests green

For bug fixes (Prove-It pattern):
1. Write a test that reproduces the bug (must FAIL)
2. Confirm the test fails
3. Implement the fix
4. Confirm the test passes
5. Run the full test suite for regressions

For browser-related issues, also read `%USERPROFILE%\.cursor\agent-skills\skills\browser-testing-with-devtools\SKILL.md` and verify with Chrome DevTools MCP if available.
