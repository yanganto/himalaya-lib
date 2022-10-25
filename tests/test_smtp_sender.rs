use std::{thread, time::Duration};

#[cfg(feature = "imap-backend")]
use himalaya_lib::{Backend, ImapBackend};
#[cfg(feature = "smtp-sender")]
use himalaya_lib::{Sender, Smtp};

#[cfg(all(feature = "imap-backend", feature = "smtp-sender"))]
#[test]
fn test_smtp_sender() {
    use himalaya_lib::{AccountConfig, ImapConfig, SmtpConfig};

    let account_config = AccountConfig::default();

    let smtp_config = SmtpConfig {
        host: "localhost".into(),
        port: 3025,
        ssl: Some(false),
        starttls: Some(false),
        insecure: Some(true),
        login: "alice@localhost".into(),
        passwd_cmd: "echo 'password'".into(),
        ..SmtpConfig::default()
    };
    let mut smtp = Smtp::new(&account_config, &smtp_config);

    let imap_config = ImapConfig {
        host: "localhost".into(),
        port: 3143,
        ssl: Some(false),
        starttls: Some(false),
        insecure: Some(true),
        login: "patrick@localhost".into(),
        passwd_cmd: "echo password".into(),
        ..ImapConfig::default()
    };
    let mut imap = ImapBackend::new(&imap_config).unwrap();

    // setting up folders
    imap.delete_email("INBOX", "1:*").unwrap();

    // checking that an email can be sent
    let email = include_bytes!("./emails/alice-to-patrick.eml");
    smtp.send(email).unwrap();

    thread::sleep(Duration::from_secs(1));

    // checking that the envelope of the sent email exists
    let envelopes = imap.list_envelope("INBOX", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    imap.disconnect().unwrap();
}
