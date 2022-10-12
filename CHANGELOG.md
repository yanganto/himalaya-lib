# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* Added pipe support for `(imap|smtp)-passwd-cmd` [github#373].
* Added `imap-ssl` and `smtp-ssl` options to be able to disable
  encryption [github#347].
* Implemented sendmail sender [github#351].
* Fixed `process` module for `MINGW*` [github#254].

### Changed

* Moved `Email::fold_text_plain_parts` to `Parts::to_readable`. It
  take now a `PartsReaderOptions` as parameter:
  
  * `plain_first`: shows plain texts first, switch to html if empty.
  
  * `sanitize`: sanitizes or not text bodies (both plain and html).

### Fixed

* Fixed long subject decoding issue [github#380].
* Fixed bad mailbox name encoding from UTF7-IMAP [github#370].

## [0.3.1] - 2022-10-10

### Changed

* Renamed `EmailSendCmd` into `SendmailConfig`.
* Renamed `EmailSender::Cmd` into `EmailSender::Sendmail`.

### Fixed

* Fixed broken tests

### Removed

* Removed useless dependency `toml` [patch#1].
  
## [0.3.0] - 2022-10-10

### Changed

* Renamed `DEFAULT_DRAFT_FOLDER` to `DEFAULT_DRAFTS_FOLDER` to be more
  consistant with IMAP folder names.
* Changed licence to `MIT`.
* Renamed feature `internal-sender` to `smtp-sender`.
  
### Fixed

* Fixed folder name case (because IMAP folders are case sensitive).

## [0.2.1] - 2022-09-29

### Changed

* Removed notmuch from the default features.

## [0.2.0] - 2022-09-28

### Changed

* Unwrapped folders and envelopes from struct:

  ```rust
  // Before
  pub struct Envelopes {
	  pub envelopes: Vec<Envelope>,
  }
  
  // After
  pub struct Envelopes(pub Vec<Envelope>);
  ```

* Renamed `TplOverride::sig` to `TplOverride::signature`.
* Upgraded Nix deps.

### Fixed

* Fixed imap backend pagination overflow.

## [0.1.0] - 2022-09-22

First official version of the Himalaya's library. The source code
mostly comes from the [CLI](https://github.com/soywod/himalaya)
repository.

[patch#1]: https://lists.sr.ht/~soywod/himalaya-lib/%3C20220929084520.98165-1-me%40paulrouget.com%3E

[github#254]: https://github.com/soywod/himalaya/issues/254
[github#347]: https://github.com/soywod/himalaya/issues/347
[github#351]: https://github.com/soywod/himalaya/issues/351
[github#370]: https://github.com/soywod/himalaya/issues/370
[github#373]: https://github.com/soywod/himalaya/issues/373
[github#380]: https://github.com/soywod/himalaya/issues/380
