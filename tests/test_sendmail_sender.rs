use std::{thread, time::Duration};

use himalaya_lib::Email;
#[cfg(feature = "imap-backend")]
use himalaya_lib::{Backend, ImapBackend, Sender, Sendmail};

#[cfg(feature = "imap-backend")]
#[test]
fn test_sendmail_sender() {
    use himalaya_lib::{AccountConfig, ImapConfig, SendmailConfig};

    let account_config = AccountConfig::default();
    let sendmail_config = SendmailConfig {
        cmd: [
            "msmtp",
            "--host localhost",
            "--port 3025",
            "--user=alice@localhost",
            "--passwordeval='echo password'",
            "--read-envelope-from",
            "--read-recipients",
        ]
        .join(" "),
    };
    let imap_config = ImapConfig {
        host: "localhost".into(),
        port: 3143,
        ssl: Some(false),
        login: "patrick@localhost".into(),
        passwd_cmd: "echo 'password'".into(),
        ..ImapConfig::default()
    };

    let mut sendmail = Sendmail::new(&account_config, &sendmail_config);
    let mut imap = ImapBackend::new(&account_config, &imap_config);
    imap.connect().unwrap();

    // setting up folders
    imap.email_delete("INBOX", "1:*").unwrap();

    // checking that an email can be sent
    let email = Email::from_tpl(include_str!("./emails/alice-to-patrick.eml")).unwrap();
    sendmail.send(&email).unwrap();

    thread::sleep(Duration::from_secs(1));

    // checking that the envelope of the sent email exists
    let envelopes = imap.envelope_list("INBOX", 10, 0).unwrap();
    assert_eq!(1, envelopes.len());
    let envelope = envelopes.first().unwrap();
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    imap.disconnect().unwrap();
}
