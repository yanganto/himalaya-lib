use env_logger;
use std::{
    borrow::Cow,
    env::temp_dir,
    fs::{create_dir_all, remove_dir_all},
    thread,
    time::Duration,
};

use himalaya_lib::{
    imap::ImapBackendBuilder, sync, AccountConfig, Backend, CompilerBuilder, Flag, Flags,
    ImapConfig, MaildirBackend, MaildirConfig, TplBuilder,
};

#[test]
fn test_sync() {
    env_logger::builder().is_test(true).init();

    // set up account

    let sync_dir = temp_dir().join("himalaya-sync");
    if sync_dir.is_dir() {
        remove_dir_all(&sync_dir).unwrap();
    }
    create_dir_all(&sync_dir).unwrap();

    let account = AccountConfig {
        name: "account".into(),
        sync: true,
        sync_dir: Some(sync_dir.clone()),
        ..AccountConfig::default()
    };

    // set up imap backend

    let imap = ImapBackendBuilder::default()
        .pool_size(10)
        .build(
            Cow::Borrowed(&account),
            Cow::Owned(ImapConfig {
                host: "localhost".into(),
                port: 3143,
                ssl: Some(false),
                starttls: Some(false),
                insecure: Some(true),
                login: "bob@localhost".into(),
                passwd_cmd: "echo 'password'".into(),
                ..ImapConfig::default()
            }),
        )
        .unwrap();

    // purge folders

    if let Err(_) = imap.add_folder("Sent") {};
    imap.purge_folder("INBOX").unwrap();
    imap.purge_folder("Sent").unwrap();

    // add 3 emails with delay (in order to have a different date)

    imap.add_email(
        "INBOX",
        &TplBuilder::default()
            .message_id("<a@localhost>")
            .from("alice@localhost")
            .to("bob@localhost")
            .subject("A")
            .text_plain_part("A")
            .compile(CompilerBuilder::default())
            .unwrap(),
        &Flags::default(),
    )
    .unwrap();

    thread::sleep(Duration::from_secs(1));

    imap.add_email(
        "INBOX",
        &TplBuilder::default()
            .message_id("<b@localhost>")
            .from("alice@localhost")
            .to("bob@localhost")
            .subject("B")
            .text_plain_part("B")
            .compile(CompilerBuilder::default())
            .unwrap(),
        &Flags::from_iter([Flag::Flagged]),
    )
    .unwrap();

    thread::sleep(Duration::from_secs(1));

    imap.add_email(
        "INBOX",
        &TplBuilder::default()
            .message_id("<c@localhost>")
            .from("alice@localhost")
            .to("bob@localhost")
            .subject("C")
            .text_plain_part("C")
            .compile(CompilerBuilder::default())
            .unwrap(),
        &Flags::default(),
    )
    .unwrap();

    let imap_envelopes = imap.list_envelopes("INBOX", 0, 0).unwrap();

    // init maildir backend reader

    let mdir = MaildirBackend::new(
        Cow::Borrowed(&account),
        Cow::Owned(MaildirConfig {
            root_dir: sync_dir.join(&account.name),
        }),
    )
    .unwrap();

    // sync imap account twice in a row to see if all work as expected
    // without duplicate items

    sync::sync(&account, &imap).unwrap();
    sync::sync(&account, &imap).unwrap();

    // check maildir envelopes integrity

    let mdir_envelopes = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    assert_eq!(*imap_envelopes, *mdir_envelopes);

    // check maildir emails content integrity

    let ids = mdir_envelopes.iter().map(|e| e.id.as_str()).collect();
    let emails = mdir.get_emails("INBOX", ids).unwrap();
    let emails = emails.to_vec();
    assert_eq!(3, emails.len());
    assert_eq!("C\r\n", emails[0].parsed().unwrap().get_body().unwrap());
    assert_eq!("B\r\n", emails[1].parsed().unwrap().get_body().unwrap());
    assert_eq!("A\r\n", emails[2].parsed().unwrap().get_body().unwrap());

    // check cache integrity

    let cache = sync::Cache::new(Cow::Borrowed(&account), &sync_dir).unwrap();

    let cached_mdir_envelopes = cache.list_local_envelopes("INBOX").unwrap();
    assert_eq!(cached_mdir_envelopes, mdir_envelopes);

    let cached_imap_envelopes = cache.list_remote_envelopes("INBOX").unwrap();
    assert_eq!(cached_imap_envelopes, imap_envelopes);

    // remove emails and update flags from both side, sync again and
    // check integrity

    imap.delete_emails_internal("INBOX", vec![&imap_envelopes[0].internal_id])
        .unwrap();
    imap.add_flags_internal(
        "INBOX",
        vec![&imap_envelopes[1].internal_id],
        &Flags::from_iter([Flag::Draft]),
    )
    .unwrap();
    mdir.delete_emails_internal("INBOX", vec![&mdir_envelopes[2].internal_id])
        .unwrap();
    mdir.add_flags_internal(
        "INBOX",
        vec![&mdir_envelopes[1].internal_id],
        &Flags::from_iter([Flag::Flagged, Flag::Answered]),
    )
    .unwrap();

    sync::sync(&account, &imap).unwrap();

    let imap_envelopes = imap.list_envelopes("INBOX", 0, 0).unwrap();
    let mdir_envelopes = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    assert_eq!(imap_envelopes, mdir_envelopes);

    let cached_mdir_envelopes = cache.list_local_envelopes("INBOX").unwrap();
    assert_eq!(cached_mdir_envelopes, mdir_envelopes);

    let cached_imap_envelopes = cache.list_remote_envelopes("INBOX").unwrap();
    assert_eq!(cached_imap_envelopes, imap_envelopes);
}
