#[cfg(feature = "notmuch-backend")]
use concat_with::concat_line;
#[cfg(feature = "notmuch-backend")]
use std::{collections::HashMap, env, fs, iter::FromIterator};

#[cfg(feature = "notmuch-backend")]
use himalaya_lib::{
    AccountConfig, Backend, CompilerBuilder, Flag, Flags, NotmuchBackend, NotmuchConfig, TplBuilder,
};

#[cfg(feature = "notmuch-backend")]
#[test]
fn test_notmuch_backend() {
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
    let email = TplBuilder::default()
        .from("alice@localhost")
        .to("bob@localhost")
        .subject("Plain message!")
        .text_plain_part("Plain message!")
        .compile(CompilerBuilder::default())
        .unwrap();
    let flags = Flags::from_iter([Flag::custom("inbox"), Flag::Seen]);
    let hash = notmuch.add_email("", &email, &flags).unwrap();

    // check that the added message exists
    let emails = notmuch.get_emails("", vec![&hash]).unwrap();
    assert_eq!(
        concat_line!(
            "From: alice@localhost",
            "To: bob@localhost",
            "",
            "Plain message!\r\n",
        ),
        *emails
            .to_vec()
            .first()
            .unwrap()
            .to_read_tpl_builder(&account_config)
            .unwrap()
            .show_headers(["From", "To"])
            .build()
    );

    // check that the envelope of the added message exists
    let envelopes = notmuch.list_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message!", envelope.subject);

    // check that a flag can be added to the message
    let flags = Flags::from_iter([Flag::Flagged, Flag::Answered]);
    notmuch.add_flags("", vec![&envelope.id], &flags).unwrap();
    let envelopes = notmuch.list_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(envelope.flags.contains(&Flag::Seen));
    assert!(envelope.flags.contains(&Flag::Flagged));
    assert!(envelope.flags.contains(&Flag::Answered));

    // check that the message flags can be changed
    let flags = Flags::from_iter([Flag::custom("inbox"), Flag::Answered]);
    notmuch.set_flags("", vec![&envelope.id], &flags).unwrap();
    let envelopes = notmuch.list_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(envelope.flags.contains(&Flag::Answered));

    // check that a flag can be removed from the message
    let flags = Flags::from_iter([Flag::Answered]);
    notmuch
        .remove_flags("", vec![&envelope.id], &flags)
        .unwrap();
    let envelopes = notmuch.list_envelopes("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Custom("inbox".into())));
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(!envelope.flags.contains(&Flag::Answered));

    // check that the message can be deleted
    notmuch.delete_emails("", vec![&hash]).unwrap();
    assert!(notmuch.get_emails("inbox", vec![&hash]).is_err());
}
