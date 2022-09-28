# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2022-09-29

### Changed

- Remove notmuch from the default features

## [0.2.0] - 2022-09-28

### Changed

- Folders and Envelopes are not wrapped anymore in a struct:

  ```rust
  // Before
  pub struct Envelopes {
	  pub envelopes: Vec<Envelope>,
  }
  
  // After
  pub struct Envelopes(pub Vec<Envelope>);
  ```

- `TplOverride::sig` has been renamed `TplOverride::signature`
- Nix deps have been upgraded

### Fixed

- Imap backend pagination overflow

## [0.1.0] - 2022-09-22

First official version of the Himalaya's library. The source code
mostly comes from the [CLI](https://github.com/soywod/himalaya)
repository.
