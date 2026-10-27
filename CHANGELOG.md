# Changelog

## Unreleased

### Added

- Forklift warehouse frontend with an isolated virtual demo.
- Linux ext4 real-file adapter with Landlock confinement, journaled no-replace moves, conditional undo, verified copy, and recovery.
- Safety, crash-boundary, property, and frontend test suites.

### Limitations

- Real-file mode is disabled on macOS and Windows until their native adapters pass the same guarantees.
- No release package has been published or signed.
