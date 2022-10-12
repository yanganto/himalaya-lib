use himalaya_lib::AccountConfig;
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
    let mut imap = ImapBackend::new(&account_config, &imap_config);

    // checking that the backend can connect
    imap.connect().unwrap();

    // setting up folders
    if let Err(_) = imap.folder_add("Folder1") {};
    if let Err(_) = imap.folder_add("Folder2") {};
    imap.email_delete("INBOX", "1:*").unwrap();
    imap.email_delete("Folder1", "1:*").unwrap();
    imap.email_delete("Folder2", "1:*").unwrap();

    // checking that an email can be added
    let email = include_bytes!("./emails/alice-to-patrick.eml");
    let id = imap
        .email_add("Folder1", email, "seen")
        .unwrap()
        .to_string();

    // checking that the added email exists
    let email = imap.email_get("Folder1", &id).unwrap();
    assert_eq!("alice@localhost", email.from.clone().unwrap().to_string());
    assert_eq!("patrick@localhost", email.to.clone().unwrap().to_string());
    assert_eq!("Ceci est un message.", email.fold_text_plain_parts());

    // checking that the envelope of the added email exists
    let envelopes = imap.envelope_list("Folder1", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // checking that the email can be copied
    imap.email_copy("Folder1", "Folder2", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.envelope_list("Folder1", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelopes = imap.envelope_list("Folder2", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());

    // checking that the email can be moved
    imap.email_move("Folder1", "Folder2", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.envelope_list("Folder1", 10, 0).unwrap();
    assert_eq!(0, envelopes.len());
    let envelopes = imap.envelope_list("Folder2", 10, 0).unwrap();
    assert_eq!(2, envelopes.len());
    let id = envelopes.first().unwrap().id.to_string();

    // checking that the email can be deleted
    imap.email_delete("Folder2", &id).unwrap();
    assert!(imap.email_get("Folder2", &id).is_err());

    // checking that the backend can disconnect
    imap.disconnect().unwrap();
}
