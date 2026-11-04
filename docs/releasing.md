# Releasing

Every user-visible change gets a `CHANGELOG.md` entry and a tag. The app reads
that file at runtime and shows it in Settings as Version history, so an entry
reaches users without a rebuild of anything else.

1. Add the entry at the top of `CHANGELOG.md`, under a
   `## <version> · <YYYY-MM-DD>` heading. One bullet per change, written for
   someone using the tool, not for someone reading the diff. Name the screen
   and the control: "Settings holds Version history", not "added about panel".
2. Bump `version` in the workspace `Cargo.toml`. The app reads it through
   `CARGO_PKG_VERSION`.
3. Commit with the subject `lyra-webtool v<version>: <imperative summary>`.
4. Tag it: `git tag -a v<version>` with the same bullets as the message.
5. Push `main` and the tag. The Pages workflow builds and deploys.

`build.sh` stamps the commit and date into the binary and writes
`www/version.json`. The running app compares itself against that file, so a
user on a stale cached copy is told a newer version exists and can clear the
cache from Settings.

Bump the minor version for a new capability, the patch version for a fix.
Nothing here is at 1.0 while the flash path is unverified on hardware.
