#[cfg(feature = "maildir-backend")]
use concat_with::concat_line;
#[cfg(feature = "maildir-backend")]
use maildir::Maildir;
#[cfg(feature = "maildir-backend")]
use std::{borrow::Cow, collections::HashMap, fs, iter::FromIterator};
#[cfg(feature = "maildir-backend")]
use tempfile::tempdir;

#[cfg(feature = "maildir-backend")]
use himalaya_lib::{
    AccountConfig, Backend, CompilerBuilder, Flag, Flags, MaildirBackend, MaildirConfig, TplBuilder,
};

#[cfg(feature = "maildir-backend")]
#[test]
fn test_maildir_backend() {
    env_logger::builder().is_test(true).init();

    // set up maildir folders

    let mdir: Maildir = tempdir().unwrap().path().to_owned().into();
    if let Err(_) = fs::remove_dir_all(mdir.path()) {}
    mdir.create_dirs().unwrap();

    let mdir_sub: Maildir = mdir.path().join(".Subdir").into();
    if let Err(_) = fs::remove_dir_all(mdir_sub.path()) {}
    mdir_sub.create_dirs().unwrap();

    let account_config = AccountConfig {
        name: "account".into(),
        folder_aliases: HashMap::from_iter([("subdir".into(), "Subdir".into())]),
        ..AccountConfig::default()
    };

    let mdir = MaildirBackend::new(
        Cow::Borrowed(&account_config),
        Cow::Owned(MaildirConfig {
            root_dir: mdir.path().to_owned(),
        }),
    )
    .unwrap();

    let submdir = MaildirBackend::new(
        Cow::Borrowed(&account_config),
        Cow::Owned(MaildirConfig {
            root_dir: mdir_sub.path().to_owned(),
        }),
    )
    .unwrap();

    // check that a message can be built and added
    let email = TplBuilder::default()
        .from("alice@localhost")
        .to("bob@localhost")
        .subject("Plain message!")
        .text_plain_part("Plain message!")
        .compile(CompilerBuilder::default())
        .unwrap();
    let flags = Flags::from_iter([Flag::Seen]);
    let id = mdir.add_email("INBOX", &email, &flags).unwrap();

    // check that the added message exists
    let emails = mdir.get_emails("INBOX", vec![&id]).unwrap();
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
    let envelopes = mdir.list_envelopes("INBOX", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.from.addr);
    assert_eq!("Plain message!", envelope.subject);

    // check that a flag can be added to the message
    let flags = Flags::from_iter([Flag::Flagged]);
    mdir.add_flags("INBOX", vec![&envelope.id], &flags).unwrap();
    let envelopes = mdir.list_envelopes("INBOX", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Seen));
    assert!(envelope.flags.contains(&Flag::Flagged));

    // check that the message flags can be changed
    let flags = Flags::from_iter([Flag::Answered]);
    mdir.set_flags("INBOX", vec![&envelope.id], &flags).unwrap();
    let envelopes = mdir.list_envelopes("INBOX", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(envelope.flags.contains(&Flag::Answered));

    // check that a flag can be removed from the message
    let flags = Flags::from_iter([Flag::Answered]);
    mdir.remove_flags("INBOX", vec![&envelope.id], &flags)
        .unwrap();
    let envelopes = mdir.list_envelopes("INBOX", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(!envelope.flags.contains(&Flag::Answered));

    // check that the message can be copied
    mdir.copy_emails("INBOX", "subdir", vec![&envelope.id])
        .unwrap();
    let inbox = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    let subdir = mdir.list_envelopes("subdir", 0, 0).unwrap();
    assert_eq!(1, inbox.len());
    assert_eq!(1, subdir.len());
    assert!(mdir.get_emails("INBOX", vec![&id]).is_ok());
    assert!(mdir.get_emails("subdir", vec![&id]).is_ok());
    assert!(submdir.get_emails("INBOX", vec![&id]).is_ok());

    // check that the message can be moved
    mdir.move_emails("INBOX", "subdir", vec![&envelope.id])
        .unwrap();
    let inbox = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    let subdir = mdir.list_envelopes("subdir", 0, 0).unwrap();
    assert_eq!(0, inbox.len());
    assert_eq!(1, subdir.len());
    assert!(mdir.get_emails("INBOX", vec![&id]).is_err());
    assert!(mdir.get_emails("subdir", vec![&id]).is_ok());
    assert!(submdir.get_emails("INBOX", vec![&id]).is_ok());

    // check that the message can be deleted
    mdir.delete_emails("subdir", vec![&id]).unwrap();
    let inbox = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    let subdir = mdir.list_envelopes("subdir", 0, 0).unwrap();
    assert_eq!(0, inbox.len());
    assert_eq!(0, subdir.len());
    assert!(mdir.get_emails("subdir", vec![&id]).is_err());
    assert!(submdir.get_emails("INBOX", vec![&id]).is_err());
}
