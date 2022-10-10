# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Renamed `DEFAULT_DRAFT_FOLDER` to `DEFAULT_DRAFTS_FOLDER` to be more
  consistant with IMAP folder names.
- Changed licence to `MIT`.
- Renamed feature `internal-sender` to `smtp-sender`.
  
### Fixed

- Fixed folder name case (because IMAP folders are case sensitive).

## [0.2.1] - 2022-09-29

### Changed

- Removed notmuch from the default features.

## [0.2.0] - 2022-09-28

### Changed

- Unwrapped folders and envelopes from struct:

  ```rust
  // Before
  pub struct Envelopes {
	  pub envelopes: Vec<Envelope>,
  }
  
  // After
  pub struct Envelopes(pub Vec<Envelope>);
  ```

- Renamed `TplOverride::sig` to `TplOverride::signature`.
- Upgraded Nix deps.

### Fixed

- Fixed imap backend pagination overflow.

## [0.1.0] - 2022-09-22

First official version of the Himalaya's library. The source code
mostly comes from the [CLI](https://github.com/soywod/himalaya)
repository.
