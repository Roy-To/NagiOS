# PR42 mkdir and truncate integration

The parent is fixed truncate candidate 1fd06dde27375ce4c426266f5f5ffa80df4c2ad7. Integrate only the preserved a7 mkdir storage diff, retaining current Recovery validation and M24/legacy-gap repairs. Do not copy older status records over newer documentation or import unrelated #48 UI/kernel changes.

mkdir allocates one inode and one data block under UndoAction::Create. The parent snapshot and marker precede mutation, and explicit Recovery restores the parent before releasing child allocation. Allocation begins at RESERVED_BLOCKS so undo backup block 14 is not used for directory data. The on-disk format is unchanged.

Verification plan: independently review merged transaction/validator compatibility and shared FaultDisk helper changes; retain all four mkdir regressions and the two truncate regressions. Compile and run the exact integrated source only after the previous denied operation and current execution scope are resolved. Linux-only adapter tests and guest acceptance remain separate gates. Existing source-only or predecessor PASS results do not validate the merged source.
