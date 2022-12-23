use concat_with::concat_line;

#[cfg(feature = "maildir-backend")]
use maildir::Maildir;
#[cfg(feature = "maildir-backend")]
use std::{collections::HashMap, env, fs, iter::FromIterator};

use himalaya_lib::{AccountConfig, Backend, CompilerBuilder, Flag, TplBuilder};

#[cfg(feature = "maildir-backend")]
use himalaya_lib::{MaildirBackend, MaildirConfig};

#[cfg(feature = "maildir-backend")]
#[test]
fn test_maildir_backend() {
    // set up maildir folders
    let mdir: Maildir = env::temp_dir().join("himalaya-test-mdir").into();
    if let Err(_) = fs::remove_dir_all(mdir.path()) {}
    mdir.create_dirs().unwrap();

    let mdir_sub: Maildir = mdir.path().join(".Subdir").into();
    if let Err(_) = fs::remove_dir_all(mdir_sub.path()) {}
    mdir_sub.create_dirs().unwrap();

    let account_config = AccountConfig {
        folder_aliases: HashMap::from_iter([("subdir".into(), "Subdir".into())]),
        ..AccountConfig::default()
    };

    let mdir_config = MaildirConfig {
        root_dir: mdir.path().to_owned(),
    };
    let mdir = MaildirBackend::new(&account_config, &mdir_config);

    let submdir_config = MaildirConfig {
        root_dir: mdir_sub.path().to_owned(),
    };
    let submdir = MaildirBackend::new(&account_config, &submdir_config);

    // check that a message can be built and added
    let email = TplBuilder::default()
        .from("alice@localhost")
        .to("bob@localhost")
        .subject("Plain message!")
        .text_plain_part("Plain message!")
        .compile(CompilerBuilder::default())
        .unwrap();
    let hash = mdir.add_email("inbox", &email, "seen").unwrap();

    // check that the added message exists
    let mut email = mdir.get_email("inbox", &hash).unwrap();
    assert_eq!(
        concat_line!(
            "From: alice@localhost",
            "To: bob@localhost",
            "",
            "Plain message!\r\n",
        ),
        *email
            .to_read_tpl_builder(&account_config)
            .unwrap()
            .show_headers(["From", "To"])
            .build()
    );

    // check that the envelope of the added message exists
    let envelopes = mdir.list_envelope("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message!", envelope.subject);

    // check that a flag can be added to the message
    mdir.add_flags("inbox", &envelope.id, "flagged").unwrap();
    let envelopes = mdir.list_envelope("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Seen));
    assert!(envelope.flags.contains(&Flag::Flagged));

    // check that the message flags can be changed
    mdir.set_flags("inbox", &envelope.id, "answered").unwrap();
    let envelopes = mdir.list_envelope("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(envelope.flags.contains(&Flag::Answered));

    // check that a flag can be removed from the message
    mdir.remove_flags("inbox", &envelope.id, "answered")
        .unwrap();
    let envelopes = mdir.list_envelope("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(!envelope.flags.contains(&Flag::Answered));

    // check that the message can be copied
    mdir.copy_email("inbox", "subdir", &envelope.id).unwrap();
    assert!(mdir.get_email("inbox", &hash).is_ok());
    assert!(mdir.get_email("subdir", &hash).is_ok());
    assert!(submdir.get_email("inbox", &hash).is_ok());

    // check that the message can be moved
    mdir.move_email("inbox", "subdir", &envelope.id).unwrap();
    assert!(mdir.get_email("inbox", &hash).is_err());
    assert!(mdir.get_email("subdir", &hash).is_ok());
    assert!(submdir.get_email("inbox", &hash).is_ok());

    // check that the message can be deleted
    mdir.delete_email("subdir", &hash).unwrap();
    assert!(mdir.get_email("subdir", &hash).is_err());
    assert!(submdir.get_email("inbox", &hash).is_err());
}
