# Contributing

This guide covers the development setup, local checks and CI.

## 1. Setup

Run this once after cloning to enable the pre-push hook.

```bash
make hooks
```

## 2. Checks

Every push runs `make check` first: formatting, linting, Rust tests, installer and release tests and the build.

Run it manually with `make check`.

## 3. CI

CI runs the Linux checks on every push and PR.

Releases need an Apple Silicon build, so the macOS checks and the release run only when semantic-release plans a new version.
