use std::{
    collections::HashMap,
    env::temp_dir,
    fs::{create_dir_all, remove_dir_all},
};

use himalaya_lib::{
    sync, AccountConfig, Backend, CompilerBuilder, Flag, Flags, ImapBackend, ImapConfig,
    MaildirBackend, MaildirConfig, SyncIdMapper, TplBuilder,
};

#[test]
fn test_sync() {
    // set up account

    let sync_dir = temp_dir().join("himalaya-sync");
    if sync_dir.is_dir() {
        remove_dir_all(&sync_dir).unwrap();
    }
    create_dir_all(&sync_dir).unwrap();

    let config = AccountConfig {
        sync: true,
        sync_dir: Some(sync_dir.clone()),
        ..AccountConfig::default()
    };

    // set up imap backend

    let imap = ImapBackend::new(
        config.clone(),
        ImapConfig {
            host: "localhost".into(),
            port: 3143,
            ssl: Some(false),
            starttls: Some(false),
            insecure: Some(true),
            login: "bob@localhost".into(),
            passwd_cmd: "echo 'password'".into(),
            ..ImapConfig::default()
        },
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
                .from("alice@localhost")
                .to("bob@localhost")
                .subject("A")
                .text_plain_part("A")
                .compile(CompilerBuilder::default())
                .unwrap(),
            &Flags::default(),
        )
        .unwrap();
    let imap_internal_id_a = imap.get_envelope("INBOX", &imap_id_a).unwrap().internal_id;

    let imap_id_b = imap
        .add_email(
            "INBOX",
            &TplBuilder::default()
                .from("alice@localhost")
                .to("bob@localhost")
                .subject("B")
                .text_plain_part("B")
                .compile(CompilerBuilder::default())
                .unwrap(),
            &Flags::from_iter([Flag::Flagged]),
        )
        .unwrap();
    let imap_internal_id_b = imap.get_envelope("INBOX", &imap_id_b).unwrap().internal_id;

    let imap_id_c = imap
        .add_email(
            "INBOX",
            &TplBuilder::default()
                .from("alice@localhost")
                .to("bob@localhost")
                .subject("C")
                .text_plain_part("C")
                .compile(CompilerBuilder::default())
                .unwrap(),
            &Flags::default(),
        )
        .unwrap();
    let imap_internal_id_c = imap.get_envelope("INBOX", &imap_id_c).unwrap().internal_id;

    // init maildir backend reader

    let mdir_config = MaildirConfig {
        root_dir: sync_dir.clone(),
    };
    let mdir = MaildirBackend::new(config.clone(), mdir_config).unwrap();

    // sync imap account

    sync(&config, &imap).unwrap();

    // retrigger sync to check duplication issues

    sync(&config, &imap).unwrap();

    // check envelopes validity

    let mut envelopes = mdir.list_envelopes("INBOX", 0, 0).unwrap();
    envelopes.sort_by(|a, b| a.subject.partial_cmp(&b.subject).unwrap());
    let mut envelopes = envelopes.iter();
    assert_eq!(3, envelopes.len());

    let envelope = envelopes.next().unwrap();
    let mdir_internal_id_a = envelope.internal_id.clone();
    assert_eq!("A", envelope.subject);
    assert_eq!(Flags::from_iter([Flag::Seen]), envelope.flags);

    let envelope = envelopes.next().unwrap();
    let mdir_internal_id_b = envelope.internal_id.clone();
    assert_eq!("B", envelope.subject);
    assert_eq!(
        Flags::from_iter([Flag::Seen, Flag::Flagged]),
        envelope.flags
    );

    let envelope = envelopes.next().unwrap();
    let mdir_internal_id_c = envelope.internal_id.clone();
    assert_eq!("C", envelope.subject);
    assert_eq!(Flags::from_iter([Flag::Seen]), envelope.flags);

    let emails = mdir.get_emails("INBOX", vec![&envelope.id]).unwrap();
    let emails = emails.to_vec();
    assert_eq!(1, emails.len());

    let email = emails.first().unwrap().parsed().unwrap();
    assert_eq!("C\r\n", email.get_body().unwrap());

    // check sync id mapper file validity

    assert_eq!(
        HashMap::from_iter([
            (mdir_internal_id_a, imap_internal_id_a),
            (mdir_internal_id_b, imap_internal_id_b),
            (mdir_internal_id_c, imap_internal_id_c),
        ]),
        SyncIdMapper::new(sync_dir).unwrap().map,
    );
}
