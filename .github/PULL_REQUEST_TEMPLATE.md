# Summary

What changed, and why?

## Scope and source of truth

- Related issue, task, or workstream (if applicable):
- Owning area and changed paths checked:
- Relevant DEC, ADR, implementation specification, or acceptance document:
- For Linux migration work, cite DEC-042/043 and the assigned lane/workstream.
  Do not relabel 0.1 evidence as Linux-target acceptance.
- If this changes user-facing behavior, identify the applicable confirmed
  decision. Label new ideas as PROVISIONAL or REVIEW NOTE and leave product
  approval to the appropriate owner.

## Verification evidence

Record the exact command or scenario, source revision, environment, and result.
Mark items not run or not applicable with a short reason.

- [ ] Host-only checks: host, command, and result.
- [ ] QEMU/guest run: image or revision, guest configuration, scenario, and
  result.
- [ ] Nagi target acceptance: existing gate/workstream, owner, evidence, and
  result.
- [ ] Not run or not applicable: reason and any blocker.

A host-only result does not establish QEMU/guest behavior. A QEMU run is target
acceptance only when it satisfies the existing acceptance criteria and owner
review. Do not create or change a release gate through a pull request template.

## Limits and blockers

List what remains unverified, blocked, or dependent on another owner. Do not
report an unrun check as passing.

## Privacy, security, and provenance

- [ ] The diff and attached evidence contain no credentials, tokens, personal or
  private data, user-name-containing local paths, or chat/conversation
  identifiers.
- [ ] Any copied or adapted material and new dependencies have a documented
  source and applicable rights review ([LICENSE_SCOPE.md][license]).
- [ ] Security-sensitive details are handled through [SECURITY.md][security],
  not in this public pull request.

## Documentation

- [ ] Relevant documentation and workstream evidence are updated, or not
  applicable.

[security]: https://github.com/Roy-To/NagiOS/blob/main/SECURITY.md
[license]: https://github.com/Roy-To/NagiOS/blob/main/LICENSE_SCOPE.md
