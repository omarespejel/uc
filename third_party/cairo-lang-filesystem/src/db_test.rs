use cairo_lang_utils::Intern;

use super::FilesGroup;
use crate::cfg::{Cfg, CfgSet};
use crate::db::{
    files_group_input, set_file_override_content_keyed, CrateConfiguration,
};
use crate::flag::{Flag, FlagsGroup};
use crate::ids::{CrateLongId, Directory, FlagId, FlagLongId, SmolStrId};
use crate::test_utils::FilesDatabaseForTesting;
use crate::{override_file_content, set_crate_config};

#[test]
fn test_filesystem() {
    let mut db = FilesDatabaseForTesting::default();

    let directory = Directory::Real("src".into());
    let child_str = "child.cairo";
    let db_ref = &mut db;
    let file_id = directory.file(db_ref, child_str);
    override_file_content!(db_ref, file_id, Some("content\n".into()));

    let config = CrateConfiguration::default_for_root(directory.clone());
    let crt = CrateLongId::plain(SmolStrId::from(db_ref, "my_crate")).intern(db_ref);
    set_crate_config!(db_ref, crt, Some(config.clone()));

    let crt = CrateLongId::plain(SmolStrId::from(&db, "my_crate")).intern(&db);
    let crt2 = CrateLongId::plain(SmolStrId::from(&db, "my_crate2")).intern(&db);

    assert_eq!(db.crate_config(crt), Some(&config));
    assert!(db.crate_config(crt2).is_none());

    let file_id = directory.file(&db, child_str);
    assert_eq!(db.file_content(file_id).unwrap(), "content\n");
}

#[test]
fn test_flags() {
    let mut db = FilesDatabaseForTesting::default();

    let add_withdraw_gas_flag_id = FlagLongId(Flag::ADD_WITHDRAW_GAS.into());
    db.set_flag(
        add_withdraw_gas_flag_id.clone(),
        Some(Flag::AddWithdrawGas(false)),
    );
    let id = add_withdraw_gas_flag_id.intern(&db);

    assert_eq!(db.get_flag(id), Some(Flag::AddWithdrawGas(false)));
    assert!(
        db.get_flag(FlagId::new(&db, FlagLongId("non_existing_flag".into())))
            .is_none()
    );
}

#[test]
fn test_cfgs() {
    let mut db = FilesDatabaseForTesting::default();

    db.use_cfg(&CfgSet::from_iter([Cfg::name("test"), Cfg::kv("k", "v1")]));

    db.use_cfg(&CfgSet::from_iter([Cfg::kv("k", "v2")]));

    assert_eq!(
        db.cfg_set(),
        &CfgSet::from_iter([Cfg::name("test"), Cfg::kv("k", "v1"), Cfg::kv("k", "v2")])
    )
}

#[test]
fn keyed_override_clear_unregistered_file_is_noop() {
    let mut db = FilesDatabaseForTesting::default();
    let directory = Directory::Real("src".into());
    let file_id = directory.file(&db, "untracked.cairo");
    let file_input = db.file_input(file_id).clone();

    assert!(
        !set_file_override_content_keyed(&mut db, file_input.clone(), None),
        "clearing unregistered keyed override should be a no-op"
    );

    let overrides = files_group_input(&db)
        .keyed_file_overrides(&db)
        .clone()
        .unwrap_or_default();
    assert!(
        !overrides.contains_key(&file_input),
        "unregistered clear should not create keyed tombstone slot"
    );
}

#[test]
fn keyed_override_clear_registered_file_creates_tombstone() {
    let mut db = FilesDatabaseForTesting::default();
    let directory = Directory::Real("src".into());
    let file_input = {
        let file_id = directory.file(&db, "tracked.cairo");
        db.file_input(file_id).clone()
    };

    assert!(
        set_file_override_content_keyed(&mut db, file_input.clone(), Some("content".into())),
        "registering keyed override should mutate db"
    );
    let file_id = directory.file(&db, "tracked.cairo");
    assert_eq!(
        db.file_content(file_id).as_deref(),
        Some("content"),
        "registered keyed override should serve cached content"
    );
    assert!(
        set_file_override_content_keyed(&mut db, file_input, None),
        "clearing registered keyed override should apply tombstone"
    );
    let file_id = directory.file(&db, "tracked.cairo");
    assert!(
        db.file_content(file_id).is_none(),
        "tombstone should mask file content for registered slot"
    );
}
