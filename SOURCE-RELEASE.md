# Shipping source release: 0.5.4

This archive includes the unchanged v0.5.4 application source in src/ and the Rust host, the reader resources, locked dependencies, current build/installer scripts, tests and font-shaping tools. Historical WinUI/.NET and standalone-native hosts, QPC experiments, unused fonts/frame artwork, screenshots, binaries, backups and caches are excluded.

The current Rust host still references windows/QuranMushaf.Windows/Assets/app.ico. Only this icon is retained at that path; .NET is not required. Shared installer/audit scripts retain inactive legacy branches to avoid changing the shipping build logic.

Build prerequisites and commands are in README.md. Run npm ci and cargo fetch --locked --manifest-path windows/QuranMushaf.Rust/Cargo.toml on a fresh machine before npm run publish:windows. Inno Setup 6 is required for npm run installer:windows.

All processed runtime resources are included. Font outlines can be regenerated with npm run shape:pages; this requires Edge. Two upstream Digital Khatt layout databases are included for validation and build-digital-khatt-pages.mjs. Optional historical upstream re-import commands process:qul and import:study require separately obtained raw QUL/study downloads in work/ as specified in their scripts; they are not part of a normal build. The application and Quran text are unchanged.

SOURCE-FILES.sha256 covers files copied unchanged from the previous source snapshot. EXPORT-AUDIT.json lists excluded files and the few release documentation/package-script edits. Fonts, Quran data and third-party code retain the terms in their notices; application code is MIT.
