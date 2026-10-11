# Third-Party Notices

Quran Mushaf Reader includes or uses the following third-party software and
resources. These components are not relicensed by the project's MIT license.

## Software

- React and React DOM: MIT License
- DOMPurify: Apache License 2.0 or Mozilla Public License 2.0
- Lucide: ISC License
- StPageFlip (`page-flip`): MIT License
- Rust native host: the locked dependencies and their full license texts ship
  in `THIRD-PARTY-RUST.txt`; Rust standard-library notices ship in
  `THIRD-PARTY-RUST-LIBRARY.html`.
- Microsoft WebView2: the exact SDK loader's redistributable license ships in
  `THIRD-PARTY-RUST.txt`. The shared Evergreen Runtime uses Microsoft's terms.
- Microsoft Windows App SDK: Microsoft license terms (legacy .NET build only)
- DigitalKhatt tooling and font resources: licenses supplied by the
  DigitalKhatt projects, including MIT and SIL Open Font License 1.1 terms

Copies of license texts that ship with the web application are available in
`WebApp/licenses` after installation.

## Quran Resources

Quran text, page layout data, fonts, translations, and tafsir data are bundled
from their documented upstream resource providers, including Quranic Universal
Library and DigitalKhatt. They remain subject to their upstream terms and are
not covered by the application-code license.

Audio is streamed or downloaded on request from the providers identified in
the application. Audio recordings are not bundled in the base installer.

- Floating UI: MIT License (menu positioning and keyboard focus).
- Tabbable (used by Floating UI focus management): MIT License.
