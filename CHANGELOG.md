# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* Made backend functions accept a vector of id instead of a single id
  [#20].
* Added function `Backend::purge_folder` that removes all emails
  inside a folder.
* Added new `Backend` functions using the internal id:
  * `get_envelope_internal`: gets an envelope by its internal id
  * `add_email_internal`: adds an email and returns its internal id
  * `get_emails_internal`: gets emails by their internal id
  * `copy_emails_internal`: copies emails by their internal id
  * `move_emails_internal`: copies emails by their internal id
  * `delete_emails_internal`: copies emails by their internal id
  * `add_flags_internal`: adds emails flags by their internal id
  * `set_flags_internal`: set emails flags by their internal id
  * `remove_flags_internal`: removes emails flags by their internal id
* Added emails synchronization feature. Backends that implement the
  `ThreadSafeBackend` trait inherit the `sync` function that
  synchronizes all folders and emails with a local `Maildir` instance.
* Added `Backend::sync` function and link `ThreadSafeBackend::sync` to
  it for the IMAP and the Maildir backends.
* Added the ability to URL encode Maildir folders (in order to fix
  path collisions, for eg `[Gmail]/Sent`). Also added a
  `MaildirBackendBuilder` to facilitate the usage of the
  `url_encoded_folders` option.
* Added a process lock for `ThreadSafeBackend::sync`, this way only
  one synchronization can be performed at a time (for a same account).

### Fixed

* Used native IMAP commands `copy` and `mv`.
* Fixed maildir date envelope parsing.
* Fixed inline attachments not collected.

### Changed

* Improved `Backend` method names. Also replaced the `self mut` by a
  `RefCell`.
* Simplified the `Email` struct: there is no custom implementation
  with custom fields. Now, the `Email` struct is just a wrapper around
  `mailparse::ParsedMail`.
* Improved `Flag` structures.
* Changed `Backend` trait functions due to [#20]:
  * `list_envelope` => `list_envelopes`
  * `search_envelope` => `search_envelopes`
  * `get_email` => `get_emails`, takes now `ids: Vec<&str>` and
    returns an `Emails` structure instead of an `Email`)
  * `copy_email` => `copy_emails`, takes now `ids: Vec<&str>`.
  * `move_email` => `move_emails`, takes now `ids: Vec<&str>`.
  * `delete_email` => `delete_emails`, takes now `ids: Vec<&str>`.
  * `add_flags` takes now `ids: Vec<&str>` and `flags: &Flags`.
  * `set_flags` takes now `ids: Vec<&str>` and `flags: &Flags`.
  * `remove_flags` takes now `ids: Vec<&str>` and `flags: &Flags`.

### Removed

* The `email::Tpl` structure moved to its [own
  repository](https://git.sr.ht/~soywod/mime-msg-builder).
* Encryption and signed moved with the `email::Tpl` in its own
  repository.

## [0.4.0] - 2022-10-12

### Added

* Added pipe support for `(imap|smtp)-passwd-cmd`.
* Added `imap-ssl` and `smtp-ssl` options to be able to disable
  encryption.
* Implemented sendmail sender.
* Fixed `process` module for `MINGW*`.

### Changed

* Moved `Email::fold_text_plain_parts` to `Parts::to_readable`. It
  take now a `PartsReaderOptions` as parameter:
  
  * `plain_first`: shows plain texts first, switch to html if empty.
  
  * `sanitize`: sanitizes or not text bodies (both plain and html).

### Fixed

* Fixed long subject decoding issue.
* Fixed bad mailbox name encoding from UTF7-IMAP.

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

[#20]: https://todo.sr.ht/~soywod/himalaya/20
