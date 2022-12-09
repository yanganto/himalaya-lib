#[cfg(feature = "notmuch-backend")]
use std::{collections::HashMap, env, fs, iter::FromIterator};

#[cfg(feature = "notmuch-backend")]
use himalaya_lib::{Backend, NotmuchBackend};

#[cfg(feature = "notmuch-backend")]
#[test]
fn test_notmuch_backend() {
    use himalaya_lib::{AccountConfig, Flag, NotmuchConfig};

    // set up maildir folders and notmuch database
    let mdir: maildir::Maildir = env::temp_dir().join("himalaya-test-notmuch").into();
    if let Err(_) = fs::remove_dir_all(mdir.path()) {}
    mdir.create_dirs().unwrap();
    notmuch::Database::create(mdir.path()).unwrap();

    let account_config = AccountConfig {
        folder_aliases: HashMap::from_iter([("inbox".into(), "*".into())]),
        ..AccountConfig::default()
    };

    let notmuch_config = NotmuchConfig {
        db_path: mdir.path().to_owned(),
    };

    let notmuch = NotmuchBackend::new(&account_config, &notmuch_config).unwrap();

    // check that a message can be added
    let email = include_bytes!("./emails/alice-to-patrick.eml");
    let hash = notmuch.add_email("", email, "inbox seen").unwrap();

    // check that the added message exists
    let mut email = notmuch.get_email("", &hash).unwrap();
    assert_eq!(
        "From: alice@localhost\nTo: patrick@localhost\n\nCeci est un message.",
        *email
            .to_read_tpl_builder()
            .unwrap()
            .show_headers(["From", "To"])
            .build()
    );

    // check that the envelope of the added message exists
    let envelopes = notmuch.list_envelope("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // check that a flag can be added to the message
    notmuch
        .add_flags("", &envelope.id, "flagged answered")
        .unwrap();
    let envelopes = notmuch.list_envelope("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that the message flags can be changed
    notmuch
        .set_flags("", &envelope.id, "inbox answered")
        .unwrap();
    let envelopes = notmuch.list_envelope("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that a flag can be removed from the message
    notmuch.remove_flags("", &envelope.id, "answered").unwrap();
    let envelopes = notmuch.list_envelope("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that the message can be deleted
    notmuch.delete_email("", &hash).unwrap();
    assert!(notmuch.get_email("inbox", &hash).is_err());
}
