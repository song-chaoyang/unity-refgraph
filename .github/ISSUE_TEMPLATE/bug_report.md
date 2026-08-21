---
name: Bug report
about: Create a report to help us improve
title: "[Bug] "
labels: bug
assignees: ''
---

## Description

<!-- A clear and concise description of what the bug is. -->

## Steps to Reproduce

1. Run: `unity-refgraph index build <project>`
2. Run: `unity-refgraph refs <path> -p <project>`
3. Observe: ...

## Expected behavior

<!-- What did you expect to happen? -->

## Actual behavior

<!-- What actually happened? Include error output or panics. -->

## Environment

- OS: [e.g. macOS 14.5 / Ubuntu 22.04 / Windows 11]
- Architecture: [e.g. arm64 / x86_64]
- Rust version: `rustc --version`
- unity-refgraph version: `unity-refgraph --version`
- Unity version (of the indexed project): if known

## Additional context

- Project structure: [brief description or minimal repro]
- Index file: attach `project/.unity-refgraph/index.db` if possible
- Screenshots or log output