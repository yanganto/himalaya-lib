use maildir::Maildir;
use std::{collections::HashMap, env, fs, iter::FromIterator};

use himalaya_lib::{AccountConfig, Backend, Flag, MaildirBackend, MaildirConfig};

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
    let mut mdir = MaildirBackend::new(&account_config, &mdir_config);

    let submdir_config = MaildirConfig {
        root_dir: mdir_sub.path().to_owned(),
    };
    let mut submdir = MaildirBackend::new(&account_config, &submdir_config);

    // check that a message can be added
    let msg = include_bytes!("./emails/alice-to-patrick.eml");
    let hash = mdir.email_add("inbox", msg, "seen").unwrap();

    // check that the added message exists
    let msg = mdir.email_list("inbox", &hash).unwrap();
    assert_eq!("alice@localhost", msg.from.clone().unwrap().to_string());
    assert_eq!("patrick@localhost", msg.to.clone().unwrap().to_string());
    assert_eq!("Ceci est un message.", msg.fold_text_plain_parts());

    // check that the envelope of the added message exists
    let envelopes = mdir.envelope_list("inbox", 10, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert_eq!(1, envelopes.len());
    assert_eq!("alice@localhost", envelope.sender);
    assert_eq!("Plain message", envelope.subject);

    // check that a flag can be added to the message
    mdir.flags_add("inbox", &envelope.id, "flagged").unwrap();
    let envelopes = mdir.envelope_list("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(envelope.flags.contains(&Flag::Seen));
    assert!(envelope.flags.contains(&Flag::Flagged));

    // check that the message flags can be changed
    mdir.flags_set("inbox", &envelope.id, "answered").unwrap();
    let envelopes = mdir.envelope_list("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(envelope.flags.contains(&Flag::Answered));

    // check that a flag can be removed from the message
    mdir.flags_delete("inbox", &envelope.id, "answered")
        .unwrap();
    let envelopes = mdir.envelope_list("inbox", 1, 0).unwrap();
    let envelope = envelopes.first().unwrap();
    assert!(!envelope.flags.contains(&Flag::Seen));
    assert!(!envelope.flags.contains(&Flag::Flagged));
    assert!(!envelope.flags.contains(&Flag::Answered));

    // check that the message can be copied
    mdir.email_copy("inbox", "subdir", &envelope.id).unwrap();
    assert!(mdir.email_list("inbox", &hash).is_ok());
    assert!(mdir.email_list("subdir", &hash).is_ok());
    assert!(submdir.email_list("inbox", &hash).is_ok());

    // check that the message can be moved
    mdir.email_move("inbox", "subdir", &envelope.id).unwrap();
    assert!(mdir.email_list("inbox", &hash).is_err());
    assert!(mdir.email_list("subdir", &hash).is_ok());
    assert!(submdir.email_list("inbox", &hash).is_ok());

    // check that the message can be deleted
    mdir.email_delete("subdir", &hash).unwrap();
    assert!(mdir.email_list("subdir", &hash).is_err());
    assert!(submdir.email_list("inbox", &hash).is_err());
}
