# Security

Only the latest published version is maintained. Use the newest package from [Releases](https://github.com/marlonka/Rigometry/releases); draft builds are not supported releases.

## Report a vulnerability

Use GitHub's **Security → Report a vulnerability** when available. If that option is unavailable, contact the repository owner through an established private channel. Do not post exploit details or private hardware/file exports in a public issue. Private vulnerability reporting must be enabled before this repository is made public.

Include the version, Windows version, impact and reproduction using disposable files. Remove credentials, personal file contents, private paths and device identifiers. This is a hobby project; response times are not guaranteed.

## Security boundaries

- Hardware queries and scans do not elevate privileges, install drivers, delete files or change hardware settings.
- Reports, exports and captures require new output file names. Existing destinations are refused.
- Reparse points and cloud placeholders are excluded; a denied query remains unavailable or uses a read-only fallback.
- Installed graphics drivers and Windows APIs are trusted native code. The app does not sandbox a faulty or hostile driver or filesystem.
- Scans observe a changing filesystem. Large or slow trees consume memory and time; cancellation cannot interrupt every blocked Windows API call.
- Exports contain paths and hardware information. Review them before sharing. CSV formula prefixes are escaped.
- Builds are unsigned. Published checksums identify bytes; they do not authenticate a publisher independently.

Dependency advisories are checked in CI and before a release. A passing check cannot establish absence of unknown vulnerabilities.