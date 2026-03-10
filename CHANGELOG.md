# Changelog

## 0.2.0 - 2026-03-10

- Improve first-run indexing reliability by canonicalizing collection paths and keeping `index status` aligned with the stored index state.
- Respect `.gitignore` and default junk excludes without collapsing real repos to an empty index.
- Improve first-run search quality by excluding common generated and VCS metadata paths from default indexing.
- Add richer CLI help text for top-level commands, search modes, and output flags.
- Improve human-mode UX for empty indexes and no-result search flows with actionable hints.
- Strengthen related-file ranking to consider both symbol references and dependency definitions.
