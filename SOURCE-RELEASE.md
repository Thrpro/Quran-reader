# Current development source: 0.5.7

PATCH-NOTES-0.5.7.md is the cumulative change summary from public v0.5.0 (internal v0.5.4) to this development build, including internal 0.5.5 and 0.5.6. PATCH-NOTES-0.5.6.md remains the narrower historical development note. Measured memory results and their limits are in MEMORY-PERFORMANCE.md. Existing release tags and downloads are preserved. This working tree is not a published release.

Build on Windows x64 using Node.js 24 and the Rust MSVC toolchain: npm ci, cargo fetch --locked --manifest-path windows/QuranMushaf.Rust/Cargo.toml, then npm run publish:windows. Rust standard-library notices (rust-docs component) and Windows C++ build tools are required. Inno Setup 6 is needed only for npm run installer:windows. Normal builds use the included processed Quran resources; optional raw-data re-import commands require upstream inputs described in their scripts.

The host uses WebView2, not .NET. The historical Windows project path is retained only for Assets/app.ico, referenced by the Rust resource compiler. Preserve .gitattributes so Git does not alter resource bytes. SOURCE-FILES.sha256 covers this source snapshot, excluding itself and .git metadata. Licenses and resource notices remain applicable.
