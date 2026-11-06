# PR42 absent Trash destination source-identity validation

Parent: 03210261290c9bad823ca8267861de5506f64b83. Preserve frozen parent and evidence.
A valid journal intent with an absent generated destination is unapplied only if the original name still denotes the recorded regular-file inode and generation. Missing/reused sources must fail closed as CorruptJournal, exactly like the occupied destination path. Share the existing source-identity validation across those branches without changing journal bytes, adoption rules or lock scope. Propagate other storage errors unchanged. No repair or deletion of ambiguous intents.

Negative host fixtures: persist a valid intent, remove its source, leave destination absent, and optionally recreate source with another identity. Both must refuse Trash/synchronization, retain all bytes including journals, and continue refusing after mount/explicit VFS Recovery. Positive fixture: source identity remains live, unapplied intent stays retryable across mount, normal Trash/restore preserves identity/content.

Use exact pinned ARM64-host nightly and existing Mac x86 host target, offline jobs1 and dedicated SSD outputs. Execute RED on new tests before production change, then focused/full Runtime, formal libnagi and Recovery, lint/fmt and protected-main source identity checks. ADR must name create/mkdir, same-parent rename and delete journals and state that these are supported VFS metadata operations, not generic data repair or NH16 undo.

No upload, target, guest/VM, real-media/old-claim activity, merge, public or shared branch changes. Process-shared ownership and existing unknown-media/partial-write/damaged-journal limits remain HOLD. The same reviewer will be assigned by the parent after fixation.
