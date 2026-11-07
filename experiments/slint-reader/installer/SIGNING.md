# Building, signing and releasing the native reader

Releases are built by GitHub Actions (`.github/workflows/native-reader.yml`), not on a personal PC. That is what lets SignPath sign them: it only signs files that it can trace back to this public repository and its build.

## What the workflow does

| Event | What happens |
|---|---|
| A pull request that changes the reader | Format check, lint, tests and a release build |
| **Run workflow** (Actions tab) | The above, plus the app folder, ZIP, installer and checksums, kept as a downloadable workflow artifact for 30 days |
| A pushed tag such as `v1.0.0` | The above, plus a **draft** GitHub release with those files attached, for you to review and publish |

Every packaged build also:

- takes the Quran resources from the v0.16.0 release ZIP, pinned by its SHA-256, and verifies all 1,569 files against the resource manifest, whose SHA-256 is pinned too;
- runs `verify_contextual.py`, which checks the contextual Mushaf data on all 604 pages against the original Digital Khatt layout and word databases, and fails the build on any problem;
- runs `check_licenses.py`, which fails the build if a crate that ships in the app has no license notice;
- bundles the Visual C++ runtime from the build machine's Visual Studio.

Once SignPath is set up (below), the executable is signed before it goes into the ZIP and installer, and then the installer is signed too. Until then the same workflow produces unsigned files.

## Setting up SignPath Foundation

SignPath Foundation provides free code signing certificates for open-source projects. Check their current requirements before applying; at the time of writing they include an OSI-approved license (this project is MIT), a public repository, releases built by a CI system from that repository, multi-factor authentication for everyone who can change the code or sign, and a published code signing policy.

1. **Apply** at https://signpath.org/apply. Describe the project, link the repository and the latest release, and name who can approve signing (you).
2. **Publish the code signing policy.** When approved, add this to the README, adjusted to what SignPath asks for:

   > ## Code signing policy
   >
   > Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).
   >
   > - Committers and reviewers: [Thrpro](https://github.com/Thrpro)
   > - Approvers: [Thrpro](https://github.com/Thrpro)
   >
   > Every signed release is built by GitHub Actions from this repository. Signing requests are approved by hand.
   >
   > **Privacy:** this program does not send any information to other networked systems, except to stream or download the recitation you choose from the recitation providers named in the app.

3. **In SignPath**, once your organization exists:
   - connect GitHub as the trusted build system for the project (SignPath's GitHub integration);
   - create a project for this repository, and note its slug;
   - add two artifact configurations with these exact slugs, using the files in `signpath/`:
     - `app-exe`, from `signpath/app-exe.xml`
     - `installer`, from `signpath/installer.xml`
   - use the signing policy SignPath gives you (start with a test-signing policy if one is offered, then switch to release signing).
4. **In GitHub**, go to Settings → Secrets and variables → Actions:
   - Secrets: `SIGNPATH_API_TOKEN` (a SignPath CI user's API token)
   - Variables: `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`

   Signing switches on as soon as `SIGNPATH_ORGANIZATION_ID` is set.

## Releasing a version

1. Set the version in `experiments/slint-reader/Cargo.toml` and the matching `quran-slint-reader-prototype` entry in `Cargo.lock`.
2. Commit, then create and push a tag with the same version, for example `v1.0.0`. The workflow stops if the tag and `Cargo.toml` disagree.
3. Approve the signing requests in SignPath when asked.
4. When the workflow finishes, open the draft release on GitHub, add the release notes and publish it.

## Building locally

`package.ps1` runs the same packaging on your PC:

```powershell
./package.ps1 -Exe <path to the built exe> -Version 1.0.0 -OutDir <output folder>
```

It needs Inno Setup 6 and Visual Studio's C++ tools. Local builds are never signed.

## If the dependencies change

`licenses/` and `licenses/dependency-inventory.json` currently come from the pinned v0.16.0 resource ZIP, along with the Quran resources. If a dependency update adds a crate, `check_licenses.py` fails the build and names it, and the license notices need regenerating before release.
