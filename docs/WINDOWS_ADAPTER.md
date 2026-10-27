# Windows real-file adapter

Windows real-file mode is **not enabled**. This document is the implementation
contract for enabling it without weakening PALLET's existing confinement,
no-replace, recovery, or undo guarantees.

## What is proved today

`windows_proof` invokes `ntdll!NtSetInformationFile` with native
`FileRenameInformation` (information class 10), a retained source handle, a
retained destination-directory handle, and `ReplaceIfExists = FALSE`.
The Windows CI runner verifies that it:

- preserves bytes and NTFS identity across a move and its reverse;
- refuses a collision without modifying either file; and
- refuses an undo-style reverse move when the original name is occupied.

The proof is intentionally not used by `Warehouse`: it opens its initial paths
absolutely, so it does not establish descendant confinement.

## Adapter boundary

The Linux `sys` module is used directly by the transaction engine today. A
Windows port must first move those operations behind a platform adapter with
the same semantic operations, rather than sharing Linux file descriptors or
adding `cfg(windows)` to `warehouse`.

The Windows implementation must retain native handles for the warehouse root,
metadata directories, and every directory reached while resolving a journal
path. Names are validated by `names` and then opened one component at a time
relative to the retained parent handle. A reparse point at any component,
including the final component, is an error; it is never followed.

Required operations are:

| Operation | Windows requirement |
| --- | --- |
| Root and descendant open | `NtCreateFile`/handle-relative opening; reject reparse points and escapes |
| Metadata and journal files | Handle-relative create/open; exclusive warehouse lock with Windows sharing semantics |
| Identity | NTFS volume serial plus 64-bit file ID, captured from handles |
| Enumeration | Enumerate from a directory handle; never rediscover a child by an absolute path |
| Rename | Native class-10 `NtSetInformationFile`, retained source/destination handles, no replacement |
| Copy publish | Create exclusively below retained `.pallet/tmp`, flush data, then no-replace native rename |
| Recovery and undo | Reopen journal paths through handles and compare recorded identities before every decision |

## Durability

Windows does not expose an exact equivalent of POSIX directory `fsync` for
ordinary NTFS directory handles. The adapter must not pretend otherwise. It
will call `FlushFileBuffers` for data and journal handles, record completion
boundaries conservatively, and freeze on ambiguity. The user-facing recovery
documentation must describe that power-loss durability depends on NTFS and
storage hardware write-through behavior; no directory-sync operation may be a
successful no-op.

## Acceptance gates before enabling real-file mode

1. Native Windows tests prove root relocation and junction substitution cannot
   redirect a descendant operation.
2. Tests cover reparse points, traversal/device/ADS/trailing-dot names,
   case-only collisions, read-only files, sharing violations, and missing or
   replaced destination directories.
3. Deterministic failure injection exercises journal write/flush, rename, and
   recovery boundaries with identity-based, idempotent recovery.
4. Windows GitHub Actions runs the tests natively and retains diagnostics.
5. A packaged Tauri Windows installer is run in the VNC VM: real NTFS delivery,
   undo, collision refusal, and a forklift tip leave filesystem contents
   unchanged.

Only after all gates pass may `REAL_FILES_ADAPTER` report Windows support or
the Tauri Windows fallback expose real-file commands.
