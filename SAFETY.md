# PALLET file safety specification

This document is the contract. The forklift is a joke; the filesystem behavior is not.
Any feature that conflicts with this file stays disabled until the file is revised and
the new invariants pass tests. Gameplay polish never waives a clause.

Clause numbers (S1–S7) are referenced from code comments, tests, and
[docs/FEATURES.md](docs/FEATURES.md).

## S1 — One warehouse, with an explicit boundary

The user selects exactly one existing local directory: the warehouse root. PALLET shows
its absolute path on screen at all times. It never imports anything. Users put files
there with their normal file manager.

All user-content enumeration, reads, writes, moves, copies, undo records, temporary
copies, and incident reports stay inside this root. PALLET metadata lives in a reserved
`.pallet` child. `.pallet` is never shown as a bay and can never be selected as cargo.

PALLET does not offer IPC commands that accept arbitrary paths, shell execution,
deletion, opening files in external programs, or archive extraction. Choosing a
warehouse happens through a native folder dialog that runs in the Rust process; the
WebView never sends a path.

Exceptions to the content boundary, disclosed rather than hidden:

- Packaged application resources.
- OS-managed installation and WebView state (for example WebKitGTK caches and the
  WebView's own local storage, where PALLET keeps control and sound preferences).
- The bundled sample warehouse, which lives in PALLET's per-user application data
  directory and is itself a warehouse root with the same rules.

These are not permission to browse other user files. PALLET does not claim that the
process or the operating system never touches another path.

Roots PALLET refuses:

- The user's home directory, a filesystem root, or a system directory.
- Network shares and cloud-placeholder locations.
- Any filesystem type not on the supported list for the platform adapter.
- Roots containing nested mount points (a bay on another device is shown as
  unavailable and is never entered).

### Supported adapters

| Platform | Filesystem | Real-file mode | Status |
| --- | --- | --- | --- |
| Linux x86_64 (kernel ≥ 5.6) | ext4 | Enabled | Adapter implemented and tested (see [docs/TEST_PLAN.md](docs/TEST_PLAN.md)) |
| macOS | APFS | **Disabled** | No adapter yet. Sample warehouse only. |
| Windows | NTFS | **Disabled** | No adapter yet. Sample warehouse only. |
| Browser demo | none | **Never** | Virtual in-memory files. Cannot touch local files. |

If support cannot be established at runtime (unsupported kernel, `openat2` blocked by a
sandbox, wrong filesystem), PALLET offers an isolated virtual sample warehouse and
performs no real-file operations.

### How the boundary is enforced (Linux)

String-prefix checks and "canonicalize, then use ordinary paths" are not used anywhere.

1. The root is opened once as a directory handle (`O_PATH | O_DIRECTORY | O_NOFOLLOW`).
   Its identity (device, inode) is recorded.
2. Every descendant is resolved relative to that handle with
   `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV)`.
   The kernel refuses `..` escapes, absolute paths, any symlink in any component, and
   crossing onto another mount.
3. Names are validated before use: no empty names, `.`, `..`, `/`, NUL, or the reserved
   `.pallet` name at the root.
4. Mutations use `*at` system calls on directory handles, never full path strings.
5. At commit, the root and the involved directories are re-resolved from the root
   handle, and their identities must match what was validated. The root's original
   absolute path must still resolve to the same identity; if it does not, the
   warehouse is suspended (S4, root replaced).

**Residual race, stated plainly:** Linux has no "rename only if both directories are
still beneath this root" primitive. If another program moves a bay directory *itself*
out of the warehouse in the microseconds between PALLET's revalidation and its rename,
the file lands in that directory at its new location. PALLET re-checks identities
after the rename, detects this, records it in the journal, and freezes further
operations with an explanation. It does not try to repair it. Symlink and path-component
substitution races are prevented, not just detected, because no path string is
re-resolved at rename time.

## S2 — Never destroy user content

There is no delete, trash, overwrite, truncate, or format operation. Collisions never
replace existing content. A tipped load, collision, or failed delivery cannot change
file bytes or names. No game event can call a content mutation command directly.

Precise guarantee: **PALLET never intentionally discards user content.** A successful
move changes a file's directory entry. PALLET does not promise that "no directory entry
is ever removed", because a move by definition removes the old entry.

This is a required software invariant, not a claim that the software is bug-free or
that hardware failure can be prevented. External programs, media failure, and
filesystem corruption are outside the guarantee. Where PALLET can detect ambiguity, it
stops without destructive repair.

## S3 — Same-volume delivery and crash recovery

V1 moves regular files one at a time, within the root, on the same volume. Directories
are never moved. While the forklift drives, the file stays at its source. The crate is
a picture of the file, not an intermediate location.

All mutations are serialized through one backend worker. Commit protocol:

1. **Validate**: source identity, regular file, single hard link, permissions,
   destination directory identity, root confinement, and a destination name that does
   not exist. An exclusive lock on `.pallet/lock` (`flock`) means a second PALLET
   instance cannot mutate the same warehouse.
2. **PREPARED**: append a journal record with transaction ID, operation, source and
   destination relative paths, source identity, and a content fingerprint (size and
   modification time). `fdatasync` the journal. If writing or syncing fails, nothing
   moves.
3. **Rename**: `renameat2(src_dir_fd, name, dst_dir_fd, name, RENAME_NOREPLACE)`.
   An existence check followed by a replacing rename is forbidden. If the kernel or
   filesystem does not support `RENAME_NOREPLACE` (`EINVAL`), the move is refused.
   There is no link/unlink fallback.
4. **Flush, then COMMITTED**: `fsync` both affected directories, then append and sync
   COMMITTED. Only after that does the renderer receive success.
5. **Crash windows**: if the process dies before the rename, the file is at its
   source. If it dies after, the same inode is at the destination. A process crash
   cannot create a second physical copy or remove the only one. On next open, PALLET
   runs recovery before accepting any job.

PALLET does not claim power-loss durability from an ordinary rename. The tests in this
repository kill the process at each step; they do not simulate power loss. See
"What is and isn't tested" below.

**Recovery** compares recorded identities (device + inode), never filenames alone:

| Found after restart | Decision |
| --- | --- |
| Source only, identity matches | `NOT_MOVED`. Nothing to do. |
| Destination only, identity matches | `RECOVERED_COMMITTED`. Finish the journal. |
| Both, neither, identity mismatch, torn journal record in the middle, or changed root | `FROZEN`. Explain the evidence. Delete nothing. Guess nothing. |

Recovery decisions are journaled. Running recovery repeatedly produces the same result.

The exactly-one invariant applies to each preexisting regular file under same-volume
moves, absent external mutation. Copy intentionally creates a second file (S6). Files
with more than one hard link are refused as cargo so they can't silently break this
accounting.

## S4 — Required edge behavior

| Situation | Behavior |
| --- | --- |
| Destination name exists | Refuse. Offer a user-confirmed alternate name. The commit still uses `RENAME_NOREPLACE`, so a name that appears late is also refused. Case and Unicode equivalence follow the filesystem (ext4 is case-sensitive unless a directory has the casefold attribute; casefolded directories are treated as unsupported). |
| Symlink, mount, socket, FIFO, device, or other special file | Shown as an unavailable marker. Never traversed or transported. |
| Read-only file or unwritable directory | Refused. PALLET never chmods, takes ownership, elevates, or clears a read-only flag. Product rule: a file without owner-write permission is not cargo, even though Unix would allow renaming it. |
| File open elsewhere | Never force-closed. Linux allows renaming an open file without changing its bytes; PALLET does so. PALLET does not promise to detect open handles. |
| Source changes during handling | If identity, size, or modification time differ from what was loaded, the delivery is cancelled. Copy requires the source to be unchanged before and after streaming. |
| Source or destination disappears or is replaced | Refuse, or enter recovery. Never recreate a directory, never redirect a delivery. |
| Disk full, journal I/O error, permission error | Before the rename: refuse, source untouched. After the rename: reconcile from identities. No success animation without a COMMITTED record. |
| Root renamed, detached, or replaced | Suspend. Never bind to a new directory sitting at the old path. |
| Cross-device destination (`EXDEV`) | Refuse (S5). |

## S5 — Cross-drive moves are excluded from v1

A same-volume rename and a copy-then-delete are different operations. Across drives,
the source must keep existing while the copy is built, and after the copy is published
both exist until the source is removed. "Exactly one physical copy at every instant" is
incompatible with crash-safe copy-then-delete. PALLET does not hide this with wording.

V1 refuses cross-volume destinations and nested mounts (`RESOLVE_NO_XDEV`, device
checks, and `EXDEV` all lead to refusal). There is no fallback.

A future cross-volume move would follow this protocol, and only after this file is
revised: journal intent; create an exclusively named private temporary copy on the
destination volume; stream and flush; verify content and required metadata from a
stable source; publish without replacement; record publication durably; revalidate the
source; only then remove the source entry and record completion. A crash between
publication and source removal leaves two complete copies; recovery keeps both when
uncertain. That protocol can never claim exactly-one physical storage. Supporting a
second root would also require a new scope design, because it breaks S1.

## S6 — Undo, copy, and audit

The journal (`.pallet/journal.log`) records every attempted mutation and every recovery
decision: transaction ID, timestamp, operation, relative paths, identities, state, error
code, and batch ID. Records are length-prefixed and CRC32-checksummed, so a torn tail
is detected. File contents never go into the journal. Intent is synced before the
mutation.

A **torn tail** (an incomplete last record) can only come from a crash during an
append. The last record before it tells PALLET which transaction was in flight, and
identity reconciliation settles that transaction. PALLET still freezes and asks before
continuing; continuing copies the damaged bytes to `.pallet/evidence/` and starts a new
journal segment. A bad record **in the middle** of the journal freezes the warehouse
with no automatic way forward.

**Undo Move** is a new journaled, confined, `RENAME_NOREPLACE` reverse rename. It is
refused if either identity changed, the old location is occupied, or the old directory
is gone. The user may pick an explicitly confirmed alternate name. Undo is conditional;
it is not a backup.

**Copy** (the duplicator) writes to an exclusive temporary file inside `.pallet/tmp/`,
streams from the source, `fsync`s, verifies size and a content hash against a second
read of the source, checks that the source identity, size, and modification time did
not change, then publishes with `RENAME_NOREPLACE`. An interrupted copy leaves only a
temporary file in `.pallet/tmp/`, which is never shown as cargo and is never deleted
automatically. **Undo Copy** moves the copy into a visible `Recovered Copies` bay; it
never deletes it.

PALLET never prunes the journal or recovery storage automatically.

**Incident reports** are saved only when the user asks, as new files created with
`O_EXCL` at the warehouse root. They are journaled. They are never overwritten.

## S7 — Game and backend separation

The renderer receives opaque cargo and bay IDs. It sends delivery requests. The Rust
service owns paths, validation, the journal, identity checks, and every mutation.
Filenames are displayed as plain text only, never as HTML. Physics cannot authorize a
move, retry an uncertain commit, or bypass validation.

Every request carries a transaction ID generated by the client. Replaying a transaction
ID returns its recorded status; it never performs a second mutation. A missing
acknowledgment leads to a status query, not another move.

A pending delivery cannot be cancelled by resetting or tipping the forklift.

## What is and isn't tested

Tested (on Linux/ext4, in CI and locally), see [docs/TEST_PLAN.md](docs/TEST_PLAN.md):
process termination at every commit step, journal write failure and real full-disk
volumes, missing destinations, concurrent name collisions, symlink and directory
substitution, root replacement, hard links, special files, read-only files, torn and
corrupt journals, repeated recovery, duplicate transaction IDs, second instances,
partial pallet failures, and cross-device refusal.

Not tested, and therefore not claimed: power loss or kernel crash (no VM-reset fault
testing exists yet), behavior on filesystems other than ext4, and anything on macOS or
Windows real files.
