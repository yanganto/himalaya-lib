use concat_with::concat_line;

use himalaya_lib::{AccountConfig, Backend, CompilerBuilder, TplBuilder};

#[cfg(feature = "imap-backend")]
use himalaya_lib::{ImapBackend, ImapConfig};

#[cfg(feature = "imap-backend")]
#[test]
fn test_imap_backend() {
    let config = AccountConfig {
        email_reading_decrypt_cmd: Some(String::from(
            "gpg --decrypt --quiet --recipient-file ./tests/keys/bob.key",
        )),
        email_reading_verify_cmd: Some(String::from("gpgg --verify --quiet")),
        ..AccountConfig::default()
    };

    let imap_config = ImapConfig {
        host: "localhost".into(),
        port: 3143,
        ssl: Some(false),
        starttls: Some(false),
        insecure: Some(true),
        login: "bob@localhost".into(),
        passwd_cmd: "echo 'password'".into(),
        ..ImapConfig::default()
    };
    let imap = ImapBackend::new(&imap_config).unwrap();

    // setting up folders
    if let Err(_) = imap.add_folder("Sent") {};
    if let Err(_) = imap.add_folder("Отправленные") {};
    imap.purge_folder("INBOX").unwrap();
    imap.purge_folder("Sent").unwrap();
    imap.purge_folder("Отправленные").unwrap();

    // checking that an email can be built and added
    let email =
        TplBuilder::default()
            .from("alice@localhost")
            .to("bob@localhost")
            .subject("Signed and encrypted message")
            .text_plain_part(concat_line!(
                "<#part type=text/plain sign=command encrypt=command>",
                "Signed and encrypted message!",
                "<#/part>",
            ))
            .build()
            .compile(CompilerBuilder::default().pgp_encrypt_cmd(
                "gpg -aeqr <recipient> -o - --recipient-file ./tests/keys/bob.pub",
            ))
            .unwrap();

    let id = imap
        .add_email("Sent", &email, &("seen".into()))
        .unwrap()
        .to_string();

    // checking that the added email exists
    let emails = imap.get_emails("Sent", vec![&id]).unwrap();
    assert_eq!(
        concat_line!(
            "From: alice@localhost",
            "To: bob@localhost",
            "",
            "Signed and encrypted message!\r\n\r\n",
        ),
        *emails
            .parsed()
            .first()
            .unwrap()
            .to_read_tpl_builder(&config)
            .unwrap()
            .show_headers(["From", "To"])
            .show_text_parts_only(true)
            .build()
    );

    // checking that the envelope of the added email exists
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Signed and encrypted message", envelope.subject);

    // checking that the email can be copied
    imap.copy_emails("Sent", "Отправленные", vec![&envelope.id.to_string()])
        .unwrap();
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelopes = imap.list_envelope("Отправленные", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());

    // checking that the email can be moved
    imap.move_emails("Sent", "Отправленные", vec![&envelope.id.to_string()])
        .unwrap();
    let envelopes = imap.list_envelope("Sent", 10, 0).unwrap();
    assert_eq!(0, envelopes.len());
    let envelopes = imap.list_envelope("Отправленные", 10, 0).unwrap();
    assert_eq!(2, envelopes.len());
    let id = envelopes.first().unwrap().id.to_string();

    // checking that the email can be deleted
    imap.delete_emails("Отправленные", vec![&id]).unwrap();
    assert!(imap.get_emails("Отправленные", vec![&id]).is_err());

    // checking that the backend can disconnect
    imap.disconnect().unwrap();
}
