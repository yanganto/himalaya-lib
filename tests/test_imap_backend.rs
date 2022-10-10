#[cfg(feature = "imap-backend")]
use himalaya_lib::backend::{Backend, ImapBackend};

#[cfg(feature = "imap-backend")]
#[test]
fn test_imap_backend() {
    use himalaya_lib::{AccountConfig, EmailSender, ImapConfig, SmtpConfig};

    let account_config = AccountConfig {
        email_sender: EmailSender::Smtp(SmtpConfig {
            host: "localhost".into(),
            port: 3465,
            starttls: Some(false),
            insecure: Some(true),
            login: "inbox@localhost".into(),
            passwd_cmd: "echo 'password'".into(),
        }),
        ..AccountConfig::default()
    };
    let imap_config = ImapConfig {
        host: "localhost".into(),
        port: 3993,
        starttls: Some(false),
        insecure: Some(true),
        login: "inbox@localhost".into(),
        passwd_cmd: "echo 'password'".into(),
        ..ImapConfig::default()
    };
    let mut imap = ImapBackend::new(&account_config, &imap_config);
    imap.connect().unwrap();

    // set up mailboxes
    if let Err(_) = imap.folder_add("Mailbox1") {};
    if let Err(_) = imap.folder_add("Mailbox2") {};
    imap.email_delete("Mailbox1", "1:*").unwrap();
    imap.email_delete("Mailbox2", "1:*").unwrap();

    // check that a message can be added
    let msg = include_bytes!("./emails/alice-to-patrick.eml");
    let id = imap.email_add("Mailbox1", msg, "seen").unwrap().to_string();

    // check that the added message exists
    let msg = imap.email_get("Mailbox1", &id).unwrap();
    assert_eq!("alice@localhost", msg.from.clone().unwrap().to_string());
    assert_eq!("patrick@localhost", msg.to.clone().unwrap().to_string());
    assert_eq!("Ceci est un message.", msg.fold_text_plain_parts());

    // check that the envelope of the added message exists
    let envelopes = imap.envelope_list("Mailbox1", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // check that the message can be copied
    imap.email_copy("Mailbox1", "Mailbox2", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.envelope_list("Mailbox1", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelopes = imap.envelope_list("Mailbox2", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());

    // check that the message can be moved
    imap.email_move("Mailbox1", "Mailbox2", &envelope.id.to_string())
        .unwrap();
    let envelopes = imap.envelope_list("Mailbox1", 10, 0).unwrap();
    assert_eq!(0, envelopes.len());
    let envelopes = imap.envelope_list("Mailbox2", 10, 0).unwrap();
    assert_eq!(2, envelopes.len());
    let id = envelopes.first().unwrap().id.to_string();

    // check that the message can be deleted
    imap.email_delete("Mailbox2", &id).unwrap();
    assert!(imap.email_get("Mailbox2", &id).is_err());

    // check that disconnection works
    imap.disconnect().unwrap();
}
