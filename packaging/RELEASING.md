# Releasing tuitor

A tag `vX.Y.Z` on master starts [`.github/workflows/release.yml`](../.github/workflows/release.yml).
It checks the tag against the version in `Cargo.toml`, then:

1. builds `tuitor` for `x86_64` and `aarch64` Linux (static, musl) and for macOS on Apple silicon
   and Intel, and packs each with README.md, LICENSE and CHANGELOG.md, with a `.sha256` beside it;
2. makes the GitHub release, its notes the section of CHANGELOG.md for the version;
3. publishes `tuitor-dap`, `tuitor-core`, `tuitor-code` and `tuitor` to crates.io, in that order;
4. writes `Formula/tuitor.rb` in the Homebrew tap from
   [`homebrew/tuitor.rb.tmpl`](homebrew/tuitor.rb.tmpl);
5. pushes [`aur/tuitor-bin/PKGBUILD`](aur/tuitor-bin/PKGBUILD) (the release binaries) and
   [`aur/tuitor/PKGBUILD`](aur/tuitor/PKGBUILD) (built from the tag's source) to the AUR.

Steps 3 to 5 are skipped, with a notice in the run, while their secret is not set. A crate
already at the version is left, so a failed run can be run again. A version with a `-` (such as
`0.2.0-rc.1`) is a pre-release: GitHub marks it so, and the AUR is left alone.

Nix needs no step: `nix run github:DorianDevp/tuitor` builds from [`flake.nix`](../flake.nix) at
whatever ref is asked for.

## Cutting a release

1. Set the version in `[workspace.package]` and in the three `tuitor-*` entries of
   `[workspace.dependencies]` in the root `Cargo.toml`, then `cargo check` to update `Cargo.lock`.
2. In CHANGELOG.md, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` and start a new empty
   `## [Unreleased]` above it.
3. Commit (`Release X.Y.Z`), wait for CI to pass on master, then:

   ```sh
   git tag -a vX.Y.Z -m "tuitor X.Y.Z"
   git push origin vX.Y.Z
   ```

4. Check the run, the release page, `cargo install tuitor`, `brew install DorianDevp/tuitor/tuitor`
   and the AUR pages.

## What has to exist first

| for | what | where |
|---|---|---|
| crates.io | an API token with the `publish-new` and `publish-update` scopes, from crates.io → Account Settings → API Tokens | repository secret `CARGO_REGISTRY_TOKEN` |
| Homebrew | a public repository `DorianDevp/homebrew-tuitor` (any other: variable `HOMEBREW_TAP`, as `owner/homebrew-name`), with at least one commit | — |
| | a fine-grained token with Contents: read and write on that repository only | repository secret `HOMEBREW_TAP_TOKEN` |
| AUR | an AUR account with an SSH public key; the matching private key | repository secret `AUR_SSH_PRIVATE_KEY` |
| | the name and email the AUR commits carry | repository variables `AUR_USERNAME`, `AUR_EMAIL` (or secret `AUR_EMAIL`) |
| docs | Settings → Pages → Source: GitHub Actions | — |
| Nix | `flake.lock` committed: run `nix flake lock`, or take the `flake-lock` artifact of the CI run that made one | — |

The first push to the AUR makes the packages `tuitor` and `tuitor-bin` there; after that the
account that pushed maintains them.

## Screenshots and demos

[`.github/workflows/visuals.yml`](../.github/workflows/visuals.yml) makes them again when the code
of the views they show changes, as [`docs/tapes/views.toml`](../docs/tapes/views.toml) maps them,
and commits them to master. Before a release, run it by hand with "all" if the whole interface
changed.
