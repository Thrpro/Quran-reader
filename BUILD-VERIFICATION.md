# Clean source verification

The source folder was built independently after exclusions. Asset validation, all 40 unit tests, TypeScript/Vite production build, all 5 Rust tests, the Rust release build, verification of all 1581 packaged web files, Inno Setup installer compilation, and browser runtime checks passed. Runtime checks covered pages, rendering, search, Arabic/English study panels, gestures, keyboard navigation, themes, load-error recovery and absence of JavaScript errors.

Dependencies were installed from the lockfile using the existing offline npm cache. Rust compilation used the existing local Rust MSVC toolchain and dependency cache. This verifies the cleaned source, not a download/install of new compiler tools on a pristine PC. Temporary dependencies, compiled files, test screenshots and installers were removed from this export after verification. Application source and Quran resources are unchanged from v0.5.4.
