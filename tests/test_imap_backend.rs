use himalaya_lib::{AccountConfig, PartsReaderOptions};
#[cfg(feature = "imap-backend")]
use himalaya_lib::{Backend, ImapBackend, ImapConfig};

#[cfg(feature = "imap-backend")]
#[test]
fn test_imap_backend() {
    let account_config = AccountConfig::default();
    let imap_config = ImapConfig {
        host: "localhost".into(),
        port: 3143,
        ssl: Some(false),
        starttls: Some(false),
        insecure: Some(true),
        login: "patrick@localhost".into(),
        passwd_cmd: "echo 'password'".into(),
        ..ImapConfig::default()
    };
    let mut imap = ImapBackend::new(&imap_config).unwrap();

    // setting up folders
    if let Err(_) = imap.add_folder("Sent") {};
    if let Err(_) = imap.add_folder("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-") {};
    imap.delete_email("INBOX", "1:*").unwrap();
    imap.delete_email("Sent", "1:*").unwrap();
    imap.delete_email("Отправленные", "1:*").unwrap();

    // checking that an email can be added
    let email = include_bytes!("./emails/alice-to-patrick.eml");
    let id = imap.add_email("Sent", email, "seen").unwrap().to_string();

    // checking that the added email exists
    let email = imap.get_email("Sent", &id).unwrap();
    assert_eq!("alice@localhost", email.from.clone().unwrap().to_string());
    assert_eq!("patrick@localhost", email.to.clone().unwrap().to_string());
    assert_eq!(
        "Ceci est un message.",
        email.parts.to_readable(PartsReaderOptions::default())
    );

    // checking that the envelope of the added email exists
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // checking that the email can be copied
    imap.copy_email("Sent", "Отправленные", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelopes = imap.list_envelope("Отправленные", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());

    // checking that the email can be moved
    imap.move_email("Sent", "Отправленные", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(0, envelopes.len());
    let envelopes = imap.list_envelope("Отправленные", 10, 0).unwrap();
    assert_eq!(2, envelopes.len());
    let id = envelopes.first().unwrap().id.to_string();

    // checking that the email can be deleted
    imap.delete_email("Отправленные", &id).unwrap();
    assert!(imap.get_email("Отправленные", &id).is_err());

    // checking that the backend can disconnect
    imap.disconnect().unwrap();
}
