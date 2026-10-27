# PALLET

Moving files was too easy. I fixed that.

PALLET is a tiny forklift game that moves regular files between folders in one carefully confined warehouse. Drive up, lift a crate, deliver it, and confirm the move. Tip the forklift and you get fictional paperwork; your files do not spill.

![PALLET forklift carrying hello.txt in the virtual warehouse](docs/demo.gif)

> This is pre-release software. Real-file mode is currently implemented and tested only on Linux x86_64 with ext4, Linux 5.19+, and Landlock enabled. macOS, Windows, and the browser run an isolated virtual sample only.

![PALLET mark](assets/mark.svg)

## Try it

```bash
npm install
npm run dev
```

That starts the virtual browser demo: it cannot access local files. To run the native shell on a supported Linux system, install Tauri's system dependencies, then run `npm run tauri dev`.

## Controls

Arrow keys or WASD drive. `Space` brakes. `E` raises and lowers the forks. `Q` attaches or releases a pallet. Hold `Shift` for precision driving. The in-app accessible delivery control uses the same backend path as driving; controls can be remapped and sound/reduced motion can be changed in the app.

## What a delivery does

PALLET validates the selected regular file and destination, writes and syncs a journal intent, performs a no-replace same-volume rename, syncs the affected directories, and then records the commit. If anything is ambiguous, it stops instead of guessing. Undo is another conditional no-replace move; it is not a backup.

PALLET never follows links, overwrites a destination, deletes files, or falls back to copy-and-delete across drives. The selected warehouse folder is the only user-content root it can manage. See [SAFETY.md](SAFETY.md) for the contract and [docs/RECOVERY.md](docs/RECOVERY.md) for recovery behavior.

## Development

```bash
npm test
npm run build
cargo test --workspace
cargo test --workspace --features pallet-core/failpoints
```

The second Rust command runs the Linux safety, crash, and property suites. Some tests require ext4 and Landlock; unsupported hosts skip the real-file cases rather than weakening the checks. See [docs/TEST_PLAN.md](docs/TEST_PLAN.md).

## Status

This repository has no public release yet. No artifacts are signed or notarized. The [changelog](CHANGELOG.md) tracks verified work only, and [PRIVACY.md](PRIVACY.md) documents the current network claim.

## License

MIT — see [LICENSE](LICENSE).
