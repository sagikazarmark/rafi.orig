# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Drand`: a `drand-beacon` client for Restate services, with `commit_round` (journaled clock read, round
  `lead` ahead) and `await_beacon` (durable sleep, journaled and retried fetch, verification).
- `AwaitError` separating an unavailable beacon (keep the commitment) from cancellation.
- `DrandContext`, implemented for all five SDK contexts.
