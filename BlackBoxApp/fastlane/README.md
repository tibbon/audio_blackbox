fastlane documentation
----

# Installation

Make sure you have the latest version of the Xcode command line tools installed:

```sh
xcode-select --install
```

For _fastlane_ installation instructions, see [Installing _fastlane_](https://docs.fastlane.tools/#installing-fastlane)

# Available Actions

## Mac

### mac beta

```sh
[bundle exec] fastlane mac beta
```

Build Rust lib, archive, and upload to TestFlight

### mac metadata

```sh
[bundle exec] fastlane mac metadata
```

Upload metadata and screenshots to App Store Connect

### mac fetch_metadata

```sh
[bundle exec] fastlane mac fetch_metadata
```

Download current metadata from App Store Connect

### mac cancel_review

```sh
[bundle exec] fastlane mac cancel_review
```

Cancel existing review submission (reject current build)

### mac submit_review

```sh
[bundle exec] fastlane mac submit_review
```

Submit latest build for App Store review (cancels existing submission if needed)

### mac check

```sh
[bundle exec] fastlane mac check
```

Check metadata for common rejection reasons

----

This README.md is auto-generated and will be re-generated every time [_fastlane_](https://fastlane.tools) is run.

More information about _fastlane_ can be found on [fastlane.tools](https://fastlane.tools).

The documentation of _fastlane_ can be found on [docs.fastlane.tools](https://docs.fastlane.tools).
