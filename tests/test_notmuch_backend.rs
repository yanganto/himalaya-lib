#[cfg(feature = "notmuch-backend")]
use std::{collections::HashMap, env, fs, iter::FromIterator};

#[cfg(feature = "notmuch-backend")]
use himalaya_lib::backend::{Backend, MaildirBackend, NotmuchBackend};

#[cfg(feature = "notmuch-backend")]
#[test]
fn test_notmuch_backend() {
    use himalaya_lib::{
        config::{AccountConfig, Config},
        msg::Flag,
        BackendConfig, MaildirConfig, NotmuchConfig,
    };

    // set up maildir folders and notmuch database
    let mdir: maildir::Maildir = env::temp_dir().join("himalaya-test-notmuch").into();
    if let Err(_) = fs::remove_dir_all(mdir.path()) {}
    mdir.create_dirs().unwrap();
    notmuch::Database::create(mdir.path()).unwrap();

    let notmuch_config = NotmuchConfig {
        db_path: mdir.path().to_owned(),
    };

    let config = Config {
        accounts: HashMap::from_iter([(
            String::new(),
            AccountConfig {
                default: Some(true),
                folder_aliases: Some(HashMap::from_iter([("inbox".into(), "*".into())])),
                backend: BackendConfig::Notmuch(notmuch_config.clone()),
                ..AccountConfig::default()
            },
        )]),
        ..Config::default()
    };

    let mdir_config = MaildirConfig {
        root_dir: mdir.path().to_owned(),
    };

    let mut mdir = MaildirBackend::new(&config, &mdir_config);
    let mut notmuch = NotmuchBackend::new(&config, &notmuch_config, &mut mdir).unwrap();

    // check that a message can be added
    let msg = include_bytes!("./emails/alice-to-patrick.eml");
    let hash = notmuch.add_msg("", msg, "inbox seen").unwrap();

    // check that the added message exists
    let msg = notmuch.get_msg("", &hash).unwrap();
    assert_eq!("alice@localhost", msg.from.clone().unwrap().to_string());
    assert_eq!("patrick@localhost", msg.to.clone().unwrap().to_string());
    assert_eq!("Ceci est un message.", msg.fold_text_plain_parts());

    // check that the envelope of the added message exists
    let envelopes = notmuch.get_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // check that a flag can be added to the message
    notmuch
        .add_flags("", &envelope.id, "flagged answered")
        .unwrap();
    let envelopes = notmuch.get_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that the message flags can be changed
    notmuch
        .set_flags("", &envelope.id, "inbox answered")
        .unwrap();
    let envelopes = notmuch.get_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that a flag can be removed from the message
    notmuch.del_flags("", &envelope.id, "answered").unwrap();
    let envelopes = notmuch.get_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("seen".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("flagged".into())));
    assert!(!envelope.flags.contains(&Flag::Custom("answered".into())));

    // check that the message can be deleted
    notmuch.del_msg("", &hash).unwrap();
    assert!(notmuch.get_msg("inbox", &hash).is_err());
}
