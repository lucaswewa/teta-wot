# Releasing

A release is a tag, `vX.Y.Z`, on `main`. Pushing it runs [`.github/workflows/release.yml`](../.github/workflows/release.yml), which tests, builds and publishes. The checklist below comes first: the workflow can't tell whether it was followed.

## 1. Choose the version

Read `CHANGELOG.md`'s `Unreleased` section.

- **Anything under *Removed*, or a breaking *Changed*,** needs a new minor version before 1.0 (`0.1.x` → `0.2.0`). Breaking means breaking any surface in [ADR-0061's table](adr/0061-versioning-and-stability.md#decision): the `wot` crate's API, the `labthings` wire profile, configuration files and the command line, TD `id`s, settings files, problem types, or the minimum Rust version.
- **Otherwise,** only fixes mean a patch (`0.1.0` → `0.1.1`), and additions a minor version.

## 2. Prepare the branch

1. **In `CHANGELOG.md`,** rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, and add a new, empty `## [Unreleased]` above it. Every entry says what changed for users. A breaking change says how to adapt.
2. **Set `version = "X.Y.Z"`** in the root `Cargo.toml`'s `[workspace.package]`, then run `cargo check --workspace` to update `Cargo.lock`.
3. **Merge to `main`**, and wait for CI to pass, both jobs: Rust with the client suites, and "Reference fixtures reproduce".

## 3. Soak the release build

The 24-hour soak test runs on the build that will be released:

```
cargo build --release --locked -p simulated-microscope -p soak
target\release\soak --server target\release\simulated-microscope.exe --duration 24h --report soak.csv
target\release\soak --server target\release\simulated-microscope.exe --duration 1h --profile wot
```

Both must print `PASSED`. Keep the output: its summary lines go into the release notes (step 5), and `soak.csv` can be attached to the release.

## 4. Review what is distributed

1. **`THIRD_PARTY_NOTICES.md`:** check it lists everything copied into the repository since the last release (`git log vPREVIOUS..` on vendored assets, fixtures and reproduced texts), with checksums up to date.
2. **Gather the binaries' licences, and read the summary:**

   ```
   python tools/licenses.py simulated-microscope microscope-service > target\THIRD_PARTY_LICENSES.txt
   ```

   A licence kind that hasn't been seen before (anything beyond MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 or CC0) needs a decision before it ships.
3. **Try the package's instructions** (`tools/release/README.txt`) on a Windows computer without Rust, if the binaries changed in ways they describe.

## 5. Tag

```
git switch main && git pull
git tag -a vX.Y.Z -m "teta-wot X.Y.Z"
git push origin vX.Y.Z
```

The workflow then:

1. checks that the tag matches the workspace version and that the changelog has the version's section;
2. runs the tests;
3. builds `simulated-microscope.exe` and `microscope-service.exe` in release mode;
4. packages them with `microscope.json`, `demo.py`, the README, the changelog, the notices and the gathered licences;
5. archives the source;
6. writes `SHA256SUMS`;
7. publishes a GitHub release whose notes are the changelog section.

Afterwards, edit the release to add the soak test's summary (and `soak.csv`).

## If something goes wrong

- **The workflow refuses the tag:** delete it (`git push origin :refs/tags/vX.Y.Z` and `git tag -d vX.Y.Z`), fix the version or changelog, and tag again.
- **A published release is broken:** don't move its tag. Fix, and release a patch version.
