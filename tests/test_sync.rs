use chrono::{DateTime, Local};
use env_logger;
use log::LevelFilter;
use std::{
    borrow::Cow,
    env::temp_dir,
    fs::{create_dir_all, remove_dir_all},
    thread,
    time::Duration,
};

use himalaya_lib::{
    imap::ImapBackendBuilder, sync, AccountConfig, Backend, CompilerBuilder, Envelope, Flag, Flags,
    ImapConfig, MaildirBackend, MaildirConfig, TplBuilder,
};

#[test]
fn test_sync() {
    env_logger::builder()
        .is_test(true)
        .filter_level(LevelFilter::Debug)
        .init();

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

    // add 3 emails

    let imap_id_a = imap
        .add_email(
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
    let imap_a = imap.get_envelope("INBOX", &imap_id_a).unwrap();
    thread::sleep(Duration::from_secs(1));

    let imap_id_b = imap
        .add_email(
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
    let imap_b = imap.get_envelope("INBOX", &imap_id_b).unwrap();
    thread::sleep(Duration::from_secs(1));

    let imap_id_c = imap
        .add_email(
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
    let imap_c = imap.get_envelope("INBOX", &imap_id_c).unwrap();
    thread::sleep(Duration::from_secs(1));

    // init maildir backend reader

    let mdir = MaildirBackend::new(
        Cow::Borrowed(&account),
        Cow::Owned(MaildirConfig {
            root_dir: sync_dir.join(&account.name),
        }),
    )
    .unwrap();

    // sync imap account twice in a row to see if all work as expected
    // (no duplicate)

    sync::sync(&account, &imap).unwrap();
    sync::sync(&account, &imap).unwrap();

    // check maildir envelopes integrity

    let mdir_envelopes = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    assert_eq!(3, mdir_envelopes.len());

    assert_eq!("C", mdir_envelopes[0].subject);
    assert_eq!(imap_c.date, mdir_envelopes[0].date);
    assert_eq!(Flags::from_iter([Flag::Seen]), mdir_envelopes[0].flags);

    assert_eq!("B", mdir_envelopes[1].subject);
    assert_eq!(
        Flags::from_iter([Flag::Seen, Flag::Flagged]),
        mdir_envelopes[1].flags
    );
    assert_eq!(imap_b.date, mdir_envelopes[1].date);

    assert_eq!("A", mdir_envelopes[2].subject);
    assert_eq!(Flags::from_iter([Flag::Seen]), mdir_envelopes[2].flags);
    assert_eq!(imap_a.date, mdir_envelopes[2].date);

    let ids = vec![
        mdir_envelopes[2].id.as_str(),
        mdir_envelopes[1].id.as_str(),
        mdir_envelopes[0].id.as_str(),
    ];
    let emails = mdir.get_emails("INBOX", ids).unwrap();
    let emails = emails.to_vec();
    assert_eq!(3, emails.len());
    assert_eq!("A\r\n", emails[0].parsed().unwrap().get_body().unwrap());
    assert_eq!("B\r\n", emails[1].parsed().unwrap().get_body().unwrap());
    assert_eq!("C\r\n", emails[2].parsed().unwrap().get_body().unwrap());

    // check cache integrity

    let cache = sqlite::Connection::open_with_flags(
        sync_dir.join("database.sqlite"),
        sqlite::OpenFlags::new().set_read_only(),
    )
    .unwrap();

    let query = "
        SELECT id, internal_id, hash, account, folder, GROUP_CONCAT(flag) AS flags, message_id, sender, subject, date
        FROM envelopes
        WHERE account = ?
        GROUP BY hash
        ORDER BY date DESC
    ";

    let cached_mdir_envelopes = cache
        .prepare(query)
        .unwrap()
        .into_iter()
        .bind((1, format!("{}:cache", account.name).as_str()))
        .unwrap()
        .map(|row| row.unwrap())
        .map(|row| Envelope {
            id: row.read::<&str, _>("id").into(),
            internal_id: row.read::<&str, _>("internal_id").into(),
            flags: Flags::from_iter(row.read::<&str, _>("flags").split(",").map(Flag::from)),
            message_id: row.read::<&str, _>("message_id").into(),
            subject: row.read::<&str, _>("subject").into(),
            sender: row.read::<&str, _>("sender").into(),
            date: Some(
                DateTime::parse_from_rfc3339(row.read::<&str, _>("date"))
                    .unwrap()
                    .with_timezone(&Local),
            ),
        })
        .collect::<Vec<_>>();

    let cached_imap_envelopes = cache
        .prepare(query)
        .unwrap()
        .into_iter()
        .bind((1, account.name.as_str()))
        .unwrap()
        .map(|row| row.unwrap())
        .map(|row| Envelope {
            id: row.read::<&str, _>("id").into(),
            internal_id: row.read::<&str, _>("internal_id").into(),
            flags: Flags::from_iter(row.read::<&str, _>("flags").split(",").map(Flag::from)),
            message_id: row.read::<&str, _>("message_id").into(),
            subject: row.read::<&str, _>("subject").into(),
            sender: row.read::<&str, _>("sender").into(),
            date: Some(
                DateTime::parse_from_rfc3339(row.read::<&str, _>("date"))
                    .unwrap()
                    .with_timezone(&Local),
            ),
        })
        .collect::<Vec<_>>();

    println!("cached_mdir_envelopes: {:?}", cached_mdir_envelopes);
    assert_eq!(cached_mdir_envelopes, *mdir_envelopes);
    assert_eq!(cached_imap_envelopes, vec![imap_c, imap_b, imap_a]);
}
