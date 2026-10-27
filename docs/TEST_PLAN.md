# Test plan and evidence

`cargo test --workspace --features pallet-core/failpoints` exercises the Linux core. It includes no-replace collisions, source and directory substitution, root replacement, permission refusal, cross-device refusal, undo conflicts, interrupted copies, torn journals, worker confinement, process termination around journal/rename boundaries, and randomized move/undo/copy/reopen sequences. `npm test` covers the simulation and frontend behavior.

The tests use temporary ext4-compatible fixtures and skip the real-adapter suite where Linux Landlock or the required filesystem support is absent. `smallfs.rs` additionally runs a disk-full test only when `PALLET_TEST_SMALL_EXT4` names a prepared, empty writable ext4 mount. The project does not represent process termination tests as power-loss tests.
