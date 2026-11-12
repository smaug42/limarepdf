# Working on LimarePDF

How changes are made here, for people and for agents. Keep each change small and about one behaviour. A typo or a
one-line wording change needs only the last two points of "Each change".

## Each change

- **Name what it touches.** Before a feature or a substantial change, find the code that owns each rule it
  changes ([How it is built](architecture.md) says where each part lives), the other features that use that code,
  and what happens when the work fails or is cancelled. Say so in the pull request, under Impact.
- **Change the owner, not a copy.** Use the code that already decides something, or change it on purpose with all
  its callers; never add a second version of the rule beside it.
- **Test first.** Write the test that shows the problem and see it fail on the old code, then make it pass. Choose
  the tests from what the change touches: saving, undo and redo, work in the background, signatures and
  encryption, anything that removes things. For anything that removes, redacts, flattens or signs, check the saved
  file after reading it again, with core's saved-output checker (`tests/common/saved.rs`) and with Poppler where it
  can see the result: LimarePDF reading back its own output isn't enough.
- **Keep the tests to themselves.** Tests run by default use committed files only, never `dss/` or the render
  corpus (fetched, never committed). A test that needs those, a program installed separately or a long time is
  `#[ignore]`d and given a group in `tools/ignored-tests.toml`. Core's integration tests make their folders with
  `Scratch` (`tests/common/tool.rs`: under `target/tmp/tools`, removed afterwards unless the test failed) and run
  other programs through the same file, each with a deadline; not in the system's temporary folder.
- **Small, current and honest.** A change someone can review in one sitting; the docs saying what the code does
  now, in the same pull request (and the pictures in `docs/images/` taken again when the window they show changes:
  `tools/docshots/`); the pull request saying what was run, and what passed, failed or wasn't run.
- **Only what you meant.** Read `git diff --cached` before every commit: every staged file should be intended, and
  nobody else's changes lost. One behaviour change a commit, its message naming it.

## Where work happens

- One task at a time in the checkout. When a second has to be open at once, give it a worktree outside the
  repository (`git worktree add ../limarepdf-wt/<name>`) and remove it when it's done.
- Scratch (notes, logs, trial builds) stays outside the repository, and no tracked file reads from `target/`
  (`include!`, `include_str!`, `include_bytes!` or `#[path]`). `tools/check-scratch.py`, in CI, catches such a
  capture made by mistake; it is not a security boundary.

## Before a pull request

Run what CI's `lint` job runs, to save a round trip: `cargo fmt --all --check`, clippy for the three platforms,
and the repository checks in `tools/` ([Automated tests](testing.md#automated-tests) has the commands). Then fill
in the pull request template. Spelling is Canadian.

## After a batch, and before a release

Look at how the code has spread. Bring things together before adding more when one rule now lives in two or more
places, a rule has its third exception, a small change touched unrelated parts, or one kind of bug keeps coming
back. Progress is measured by clearer ownership and new tests of how parts work together, not by size.
