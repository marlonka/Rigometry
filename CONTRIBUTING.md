# Contributing

Rigometry targets Windows 11 x64. Start with the [development guide](docs/development.md) and [architecture](docs/architecture.md).

## Before changing code

- Use an issue to describe a bug or proposed behavior. Include a small reproduction and the affected version. For larger work, agree on scope before implementing it.
- Report vulnerabilities privately using [SECURITY.md](SECURITY.md).
- Keep reports and screenshots free of personal paths, serial numbers and private file names.

## Pull requests

Keep one change per pull request. Describe the observable problem, resulting behavior and checks performed. Include screenshots for visible changes, including a compact window and both themes where affected.

Run formatting, Clippy and the full Windows test suite. For scanning, exports or CLI changes, also run the release verification script. The commands are in the [development guide](docs/development.md#checks). Use Conventional Commits, for example `fix(storage): preserve partial totals after cancellation`.

## Engineering constraints

- Hardware queries and scans remain read-only. Do not add elevation, drivers, deletion or hardware control as incidental changes.
- Missing, stale, failed and zero readings are distinct. Preserve source, units and availability; never invent values to fill a field.
- Exports must not overwrite existing files. Preserve cancellation, partial totals, memory bounds and exclusions.
- Tests must protect an observable behavior or independent contract. Keep real filesystem fixtures; isolate live hardware from layout tests. Do not loosen assertions or accept screenshot changes merely to make a check pass.
- New dependencies need a purpose, license review and updated notices. Preserve upstream copyright and license text.
- AI-assisted contributions receive the same review and verification as other contributions. Do not submit code whose provenance or behavior you cannot explain.

Application contributions are made under the repository's [MIT license](LICENSE). Third-party material retains its own terms; identify its origin and include required notices. Include the source and permission for any third-party code or artwork.

This is a personal hobby project. Review and response times are not guaranteed. Keep discussion specific, respectful and focused on the work.
